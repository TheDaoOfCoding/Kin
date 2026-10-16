//! Phone to PC, end to end: a fake phone with its own certificate talks to the real server in the test process over real TLS on
//! loopback and uploads files with `POST /v1/transfers` and `PUT .../files/...`. Covered: a single file, several files and a
//! 0-byte one, a batch of 50, a file bigger than the checkpoint interval, a resume after a dropped connection (and after one that
//! was never noticed), cancel from each side, a full disk (faked with the developer hook), a name that is taken while the `.part` file
//! is moved, what is left after a restart, the 24 hour clean-up, and a WebSocket that stays quick during a big upload.
//!
//! Received files go to a folder inside the test's temporary directory. **Nothing here touches the real Downloads folder or the real
//! clipboard.** The 2 GB runs are `#[ignore]`d (`cargo test --release -- --ignored --nocapture large`).
mod common;

use std::path::Path;
use std::time::Duration;

use common::transfer::*;
use common::*;
use kin_lib::core::{Core, Event};
use kin_lib::transfers::Status;
use serde_json::{Value, json};

async fn connected(w: &World) -> Ws {
    w.pair().await;
    w.open_ws().await
}

fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> =
        std::fs::read_dir(dir).map(|r| r.map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    names.sort();
    names
}

fn offsets(r: &HttpResponse) -> Vec<(String, u64)> {
    r.json()["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["fileId"].as_str().unwrap().to_string(), f["offset"].as_u64().unwrap()))
        .collect()
}

fn files_toasts(events: &[Event]) -> Vec<(u32, String, String)> {
    events
        .iter()
        .filter_map(|e| if let Event::FilesToast { id, title, body, .. } = e { Some((*id, title.clone(), body.clone())) } else { None })
        .collect()
}

fn drain_events(w: &mut World) -> Vec<Event> {
    std::iter::from_fn(|| w.env.events.try_recv().ok()).collect()
}

/// The toast of a finished upload. It is made after the `PUT` has been answered, in a task of its own, so this waits (up to 10 s).
async fn wait_toasts(w: &mut World) -> Vec<(u32, String, String)> {
    let mut seen = Vec::new();
    for _ in 0..400 {
        seen.extend(files_toasts(&drain_events(w)));
        if !seen.is_empty() {
            return seen;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    seen
}

const MIB: u64 = 1 << 20;

#[tokio::test]
async fn one_file_arrives_under_its_own_name_with_its_bytes_and_a_toast_that_opens_it() {
    let mut w = World::new().await;
    let _ws = connected(&w).await;
    drain_events(&mut w);
    let pat = Pattern::new(1);
    let mut c = http(&w).await;
    let r = c.post_transfer("t1", &[("f1", "IMG_0001.jpg", 3_000_000)]).await;
    assert_eq!(r.status, 200, "{:?}", r.json());
    assert_eq!(offsets(&r), [("f1".to_string(), 0)]);
    let dir = w.core().received_dir();
    assert_eq!(listing(&dir), ["IMG_0001.jpg.part"], "the .part file exists as soon as the transfer does");
    let active = w.core().status_json()["transfers"]["active"].clone();
    assert_eq!(active[0]["id"], "t1");
    assert_eq!(active[0]["direction"], "in");
    assert_eq!(active[0]["totalBytes"], 3_000_000);

    let (r, _) = c.put("t1", "f1", 0, 3_000_000, &pat, None).await.unwrap();
    assert_eq!((r.status, r.json()), (200, json!({ "received": 3_000_000 })));
    assert_eq!(listing(&dir), ["IMG_0001.jpg"]);
    pat.check_file(&dir.join("IMG_0001.jpg"), 3_000_000).unwrap();

    // The toast, and its click.
    let toasts = wait_toasts(&mut w).await;
    assert_eq!(toasts.len(), 1, "{toasts:?}");
    assert_eq!((toasts[0].1.as_str(), toasts[0].2.as_str()), ("1 file from Test Phone", "IMG_0001.jpg"));
    assert!(w.core().toast_clicked(toasts[0].0));
    assert_eq!(w.env.opener.opened_files(), [dir.join("IMG_0001.jpg")]);
    assert!(!w.core().toast_clicked(toasts[0].0), "a second click does nothing");

    // status.json and the database.
    let status = w.core().status_json();
    assert_eq!(status["transfers"]["active"], json!([]));
    let recent = &status["transfers"]["recent"][0];
    assert_eq!(
        (recent["id"].as_str(), recent["status"].as_str(), recent["bytesDone"].as_u64()),
        (Some("t1"), Some("done"), Some(3_000_000))
    );
    let (row, files) = w.core().db.get_transfer("t1").unwrap().unwrap();
    assert_eq!((row.status.as_str(), row.direction.as_str(), row.total_bytes), ("done", "in", 3_000_000));
    assert!(files[0].finalized && files[0].name == "IMG_0001.jpg" && files[0].bytes_done == 3_000_000);
    assert_eq!(files[0].local_path.as_deref(), Some(dir.join("IMG_0001.jpg").to_string_lossy().as_ref()));
}

#[tokio::test]
async fn several_files_with_a_0_byte_one_come_in_any_order_and_the_toast_opens_the_folder() {
    let mut w = World::new().await;
    let _ws = connected(&w).await;
    drain_events(&mut w);
    let (pa, pb) = (Pattern::new(2), Pattern::new(3));
    let mut c = http(&w).await;
    let r = c.post_transfer("t2", &[("a", "a.bin", 1_500_000), ("e", "empty.txt", 0), ("b", "b.bin", 5)]).await;
    assert_eq!(r.status, 200);
    assert_eq!(offsets(&r), [("a".to_string(), 0), ("e".to_string(), 0), ("b".to_string(), 0)]);
    // Any order: b, the empty file, a.
    assert_eq!(c.put("t2", "b", 0, 5, &pb, None).await.unwrap().0.status, 200);
    let (r, _) = c.put("t2", "e", 0, 0, &pa, None).await.unwrap();
    assert_eq!((r.status, r.json()), (200, json!({ "received": 0 })));
    assert!(w.core().status_json()["transfers"]["active"][0]["id"] == "t2", "not done until the last file");
    assert_eq!(c.put("t2", "a", 0, 1_500_000, &pa, None).await.unwrap().0.status, 200);
    let dir = w.core().received_dir();
    assert_eq!(listing(&dir), ["a.bin", "b.bin", "empty.txt"]);
    pa.check_file(&dir.join("a.bin"), 1_500_000).unwrap();
    pb.check_file(&dir.join("b.bin"), 5).unwrap();
    assert_eq!(std::fs::metadata(dir.join("empty.txt")).unwrap().len(), 0);
    let toasts = wait_toasts(&mut w).await;
    assert_eq!(toasts.len(), 1);
    assert_eq!(toasts[0].1, "3 files from Test Phone");
    assert!(w.core().toast_clicked(toasts[0].0));
    assert_eq!(w.env.opener.opened_folders(), std::slice::from_ref(&dir));
    assert!(w.env.opener.opened_files().is_empty(), "several files open the folder, not a file");
    // Asking again for a file that is complete, with its whole size as the offset and an empty body, is a 200.
    let (r, _) = c.put("t2", "a", 1_500_000, 1_500_000, &pa, None).await.unwrap();
    assert_eq!((r.status, r.json()), (200, json!({ "received": 1_500_000 })));
    // And a POST of the same transfer again says it is all there.
    let r = c.post_transfer("t2", &[("a", "x", 1), ("b", "x", 1), ("e", "x", 1)]).await;
    assert_eq!(offsets(&r), [("a".to_string(), 1_500_000), ("b".to_string(), 5), ("e".to_string(), 0)]);
}

#[tokio::test]
async fn a_batch_of_50_files_with_repeated_names() {
    let mut w = World::new().await;
    let _ws = connected(&w).await;
    drain_events(&mut w);
    let pat = Pattern::new(4);
    let mut c = http(&w).await;
    let files: Vec<(String, String, u64)> = (0..50)
        .map(|i| (format!("f{i}"), if i % 10 == 0 { "photo.jpg".into() } else { format!("IMG_{i:04}.jpg") }, 1000 + i as u64 * 977))
        .collect();
    let refs: Vec<(&str, &str, u64)> = files.iter().map(|(a, b, c)| (a.as_str(), b.as_str(), *c)).collect();
    let r = c.post_transfer("batch", &refs).await;
    assert_eq!(r.status, 200);
    let dir = w.core().received_dir();
    assert_eq!(listing(&dir).len(), 50, "50 .part files");
    let started = std::time::Instant::now();
    for (id, _, size) in &files {
        let (r, _) = c.put("batch", id, 0, *size, &pat, None).await.unwrap();
        assert_eq!(r.status, 200, "{id}");
    }
    eprintln!("50 files in {:?}", started.elapsed());
    let names = listing(&dir);
    assert_eq!(names.len(), 50);
    assert!(names.iter().all(|n| !n.ends_with(".part")));
    for n in ["photo.jpg", "photo (1).jpg", "photo (2).jpg", "photo (3).jpg", "photo (4).jpg"] {
        assert!(names.contains(&n.to_string()), "{n} in {names:?}");
    }
    for (i, (_, name, size)) in files.iter().enumerate() {
        // The i-th file of a name takes the i-th suffix.
        if name != "photo.jpg" {
            pat.check_file(&dir.join(name), *size).unwrap();
        } else {
            let k = i / 10;
            let final_name = if k == 0 { "photo.jpg".to_string() } else { format!("photo ({k}).jpg") };
            pat.check_file(&dir.join(final_name), *size).unwrap();
        }
    }
    let toasts = wait_toasts(&mut w).await;
    assert_eq!(toasts.len(), 1);
    assert_eq!(toasts[0].1, "50 files from Test Phone");
}

#[tokio::test]
async fn a_file_bigger_than_the_checkpoint_interval_is_written_in_pieces_and_arrives_intact() {
    let w = World::new().await;
    let _ws = connected(&w).await;
    let pat = Pattern::new(5);
    let size = 100 * MIB + 12_345;
    let mut c = http(&w).await;
    assert_eq!(c.post_transfer("big", &[("f", "video.mp4", size)]).await.status, 200);
    let (r, elapsed) = c.put("big", "f", 0, size, &pat, None).await.unwrap();
    assert_eq!(r.status, 200);
    eprintln!("100 MiB upload over loopback TLS: {:.0} MB/s", mb_per_s(size, elapsed));
    pat.check_file(&w.core().received_dir().join("video.mp4"), size).unwrap();
    let (_, files) = w.core().db.get_transfer("big").unwrap().unwrap();
    assert_eq!(files[0].bytes_done as u64, size);
}

#[tokio::test]
async fn an_upload_that_breaks_resumes_where_the_pc_has_got_to() {
    let w = World::new().await;
    let _ws = connected(&w).await;
    let pat = Pattern::new(6);
    let size = 10 * MIB + 77;
    let dir = w.core().received_dir();
    let mut c = http(&w).await;
    assert_eq!(c.post_transfer("r1", &[("f", "doc.bin", size)]).await.status, 200);
    // The connection dies 4.5 MB in (not on a piece boundary): the phone just drops it.
    assert!(c.put("r1", "f", 0, size, &pat, Some(4_500_000)).await.is_none());
    drop(c);
    // A new connection: POST again, and the PC says how much it kept (the write of what had arrived is finished first).
    let mut c = http(&w).await;
    let r = c.post_transfer("r1", &[("f", "doc.bin", size)]).await;
    assert_eq!(r.status, 200);
    let offset = offsets(&r)[0].1;
    assert!(offset > 0 && offset <= 4_500_000, "the PC kept {offset} bytes");
    assert_eq!(offset, 4_500_000, "every byte that reached the PC was written");
    assert_eq!(std::fs::metadata(dir.join("doc.bin.part")).unwrap().len(), offset);
    // From the right place it finishes.
    let (r, _) = c.put("r1", "f", offset, size, &pat, None).await.unwrap();
    assert_eq!(r.status, 200, "{:?}", r.json());
    pat.check_file(&dir.join("doc.bin"), size).unwrap();
}

#[tokio::test]
async fn starting_from_the_wrong_place_is_a_409_that_says_where_the_pc_is() {
    let w = World::new().await;
    let _ws = connected(&w).await;
    let pat = Pattern::new(61);
    let mut c = http(&w).await;
    assert_eq!(c.post_transfer("wrong", &[("f", "x.bin", 4_000)]).await.status, 200);
    assert!(c.put("wrong", "f", 0, 4_000, &pat, Some(1_000)).await.is_none());
    drop(c);
    let mut c = http(&w).await;
    for wrong in [0, 999, 1_001, 4_000] {
        let (r, _) = c.put("wrong", "f", wrong, 4_000, &pat, None).await.expect("a small body: the answer arrives");
        assert_eq!(
            (r.status, r.json()["error"].clone(), r.json()["offset"].clone()),
            (409, json!("offset_mismatch"), json!(1_000)),
            "offset {wrong}"
        );
        c = http(&w).await; // the PC closes the connection after an answer it gave before reading the body
    }
}

#[tokio::test]
async fn a_connection_the_pc_has_not_noticed_is_dead_yet_is_taken_over_by_the_new_one() {
    let w = World::new().await;
    let _ws = connected(&w).await;
    let pat = Pattern::new(7);
    let size = 8 * MIB;
    let dir = w.core().received_dir();
    let mut old = http(&w).await;
    assert_eq!(old.post_transfer("take", &[("f", "x.bin", size)]).await.status, 200);
    // The old connection stalls after 3 MiB and stays open (Wi-Fi switched off, the PC hasn't noticed).
    assert!(old.put("take", "f", 0, size, &pat, Some(3 * MIB)).await.is_none());
    // The old writer has been quiet for more than a second (a POST leaves a writer that is still receiving alone).
    tokio::time::sleep(Duration::from_millis(1_150)).await;
    // The phone is back on a new connection. The POST stops the quiet old writer first, so the answer is exact.
    let mut new = http(&w).await;
    let r = new.post_transfer("take", &[("f", "x.bin", size)]).await;
    assert_eq!(r.status, 200);
    let offset = offsets(&r)[0].1;
    assert_eq!(offset, 3 * MIB);
    let (r, _) = new.put("take", "f", offset, size, &pat, None).await.unwrap();
    assert_eq!(r.status, 200);
    pat.check_file(&dir.join("x.bin"), size).unwrap();
    // The old connection was told the truth when it was taken over (its writer stopped at what had arrived).
    let answer = read_response(&mut old.stream).await.expect("the old request is answered");
    assert_eq!((answer.status, answer.json()["offset"].clone()), (409, json!(3 * MIB)));
}

#[tokio::test]
async fn a_second_put_for_a_file_that_is_being_written_takes_over_and_nothing_is_corrupted() {
    let w = World::new().await;
    let _ws = connected(&w).await;
    let pat = Pattern::new(8);
    let size = 6 * MIB;
    let mut first = http(&w).await;
    assert_eq!(first.post_transfer("race", &[("f", "r.bin", size)]).await.status, 200);
    assert!(first.put("race", "f", 0, size, &pat, Some(2 * MIB + 5)).await.is_none());
    tokio::time::sleep(Duration::from_millis(100)).await;
    // No POST this time: the new connection goes straight to the PUT with the offset it believes in. The PUT stops the old writer and
    // takes its place, and the offset is right because the old writer finished what had arrived.
    let mut second = http(&w).await;
    let (r, _) = second.put("race", "f", 2 * MIB + 5, size, &pat, None).await.unwrap();
    assert_eq!(r.status, 200, "{:?}", r.json());
    pat.check_file(&w.core().received_dir().join("r.bin"), size).unwrap();
}

#[tokio::test]
async fn delete_drops_the_partial_files_and_keeps_the_complete_ones() {
    let w = World::new().await;
    let _ws = connected(&w).await;
    let pat = Pattern::new(9);
    let dir = w.core().received_dir();
    let mut c = http(&w).await;
    assert_eq!(c.post_transfer("d1", &[("a", "done.bin", 1000), ("b", "partial.bin", 5 * MIB)]).await.status, 200);
    assert_eq!(c.put("d1", "a", 0, 1000, &pat, None).await.unwrap().0.status, 200);
    assert!(c.put("d1", "b", 0, 5 * MIB, &pat, Some(2 * MIB)).await.is_none());
    let mut c2 = http(&w).await;
    assert_eq!(c2.simple("DELETE", "/v1/transfers/d1").await.status, 204);
    assert_eq!(listing(&dir), ["done.bin"], "the complete file stays, the partial one goes");
    assert_eq!(c2.simple("DELETE", "/v1/transfers/d1").await.status, 404, "a retried DELETE finds nothing");
    let (r, _) = c2.put("d1", "b", 0, 10, &pat, None).await.unwrap();
    assert_eq!(r.status, 404);
    let (_, files) = w.core().db.get_transfer("d1").unwrap().unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(w.core().db.get_transfer("d1").unwrap().unwrap().0.status, "canceled");
}

#[tokio::test]
async fn the_user_canceling_on_the_pc_tells_the_phone_with_a_cancel_message() {
    let w = World::new().await;
    let mut ws = connected(&w).await;
    let pat = Pattern::new(10);
    let dir = w.core().received_dir();
    let mut c = http(&w).await;
    assert_eq!(c.post_transfer("c1", &[("f", "x.bin", 5 * MIB)]).await.status, 200);
    assert!(c.put("c1", "f", 0, 5 * MIB, &pat, Some(MIB)).await.is_none());
    let core = w.env.core.clone();
    assert!(core.cancel_transfer("c1").await);
    let msg = next_json(&mut ws).await;
    assert_eq!((msg["type"].as_str(), msg["transferId"].as_str(), msg["reason"].as_str()), (Some("cancel"), Some("c1"), Some("user")));
    send_json(&mut ws, &json!({ "type": "ack", "ref": msg["id"] })).await;
    assert!(listing(&dir).is_empty(), "the partial file is gone");
    let mut c2 = http(&w).await;
    let (r, _) = c2.put("c1", "f", 0, 10, &pat, None).await.unwrap();
    assert_eq!(r.status, 404);
    assert!(!core.cancel_transfer("c1").await, "nothing left to cancel");
}

#[tokio::test]
async fn the_phone_canceling_with_a_message_stops_an_upload_too() {
    let w = World::new().await;
    let mut ws = connected(&w).await;
    let pat = Pattern::new(11);
    let dir = w.core().received_dir();
    let mut c = http(&w).await;
    assert_eq!(c.post_transfer("c2", &[("f", "x.bin", 5 * MIB)]).await.status, 200);
    assert!(c.put("c2", "f", 0, 5 * MIB, &pat, Some(MIB)).await.is_none());
    send_json(&mut ws, &json!({ "type": "cancel", "id": "m-cancel-1", "transferId": "c2", "reason": "user" })).await;
    assert_eq!(next_json(&mut ws).await, json!({ "type": "ack", "ref": "m-cancel-1" }));
    let core = w.env.core.clone();
    wait_until("the transfer to be dropped", || core.transfers.get("c2").is_none()).await;
    assert!(listing(&dir).is_empty());
    // A cancel or complete for something the PC doesn't know is acked and ignored.
    for (i, t) in ["cancel", "complete"].iter().enumerate() {
        let id = format!("m-unknown-{i}");
        send_json(&mut ws, &json!({ "type": t, "id": id, "transferId": "nope", "reason": "user" })).await;
        assert_eq!(next_json(&mut ws).await, json!({ "type": "ack", "ref": id }));
    }
}

#[tokio::test]
async fn a_full_disk_is_507_and_only_the_missing_bytes_count() {
    let w = World::new().await;
    let _ws = connected(&w).await;
    let pat = Pattern::new(12);
    let mut c = http(&w).await;
    w.core().set_fake_free_bytes(Some(4_999));
    let r = c.post_transfer("full", &[("f", "x.bin", 5_000)]).await;
    assert_eq!((r.status, r.json()["error"].clone()), (507, json!("disk_full")));
    assert!(w.core().transfers.get("full").is_none(), "nothing was created");
    assert!(listing(&w.core().received_dir()).is_empty());
    w.core().set_fake_free_bytes(Some(5_000));
    assert_eq!(c.post_transfer("full", &[("f", "x.bin", 5_000)]).await.status, 200, "5,000 free is enough for 5,000");
    assert!(c.put("full", "f", 0, 5_000, &pat, Some(3_000)).await.is_none());
    drop(c);
    let mut c = http(&w).await;
    // 2,000 bytes are missing now: that is what has to fit.
    w.core().set_fake_free_bytes(Some(1_999));
    let r = c.post_transfer("full", &[("f", "x.bin", 5_000)]).await;
    assert_eq!(r.status, 507, "a resumed POST is checked again");
    w.core().set_fake_free_bytes(Some(2_000));
    let r = c.post_transfer("full", &[("f", "x.bin", 5_000)]).await;
    assert_eq!(r.status, 200, "{:?}", r.json());
    assert_eq!(offsets(&r)[0].1, 3_000);
    // The disk fills up before the PUT: 507, and nothing is written.
    w.core().set_fake_free_bytes(Some(10));
    let (r, _) = c.put("full", "f", 3_000, 5_000, &pat, None).await.unwrap();
    assert_eq!(r.status, 507);
    assert_eq!(std::fs::metadata(w.core().received_dir().join("x.bin.part")).unwrap().len(), 3_000);
    w.core().set_fake_free_bytes(None);
    let (r, _) = c.put("full", "f", 3_000, 5_000, &pat, None).await.unwrap();
    assert_eq!(r.status, 200);
    pat.check_file(&w.core().received_dir().join("x.bin"), 5_000).unwrap();
}

#[tokio::test]
async fn a_name_taken_while_the_part_file_is_moved_gets_the_next_suffix_and_the_other_file_is_untouched() {
    let mut w = World::new().await;
    let _ws = connected(&w).await;
    drain_events(&mut w);
    let pat = Pattern::new(13);
    let dir = w.core().received_dir();
    let mut c = http(&w).await;
    assert_eq!(c.post_transfer("n1", &[("f", "a.txt", 100)]).await.status, 200);
    // Something else appears under the final name while the transfer is under way.
    std::fs::write(dir.join("a.txt"), "somebody else's file").unwrap();
    assert_eq!(c.put("n1", "f", 0, 100, &pat, None).await.unwrap().0.status, 200);
    assert_eq!(listing(&dir), ["a (1).txt", "a.txt"]);
    assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "somebody else's file");
    pat.check_file(&dir.join("a (1).txt"), 100).unwrap();
    let (_, files) = w.core().db.get_transfer("n1").unwrap().unwrap();
    assert_eq!(files[0].name, "a (1).txt", "the stored final name is the one it ended up with");
    let toasts = wait_toasts(&mut w).await;
    assert!(toasts[0].2.contains("a (1).txt"));
    assert!(w.core().toast_clicked(toasts[0].0));
    assert_eq!(w.env.opener.opened_files(), [dir.join("a (1).txt")]);

    // Two transfers in progress with the same name each keep their own claim.
    assert_eq!(c.post_transfer("n2", &[("f", "b.txt", 10)]).await.status, 200);
    assert_eq!(c.post_transfer("n3", &[("f", "b.txt", 10)]).await.status, 200);
    assert_eq!(c.put("n3", "f", 0, 10, &pat, None).await.unwrap().0.status, 200);
    assert_eq!(c.put("n2", "f", 0, 10, &pat, None).await.unwrap().0.status, 200);
    let names = listing(&dir);
    assert!(names.contains(&"b.txt".to_string()) && names.contains(&"b (1).txt".to_string()), "{names:?}");
}

#[tokio::test]
async fn names_from_the_phone_are_cleaned_and_a_name_that_cant_be_a_file_name_is_made_into_one() {
    let w = World::new().await;
    let _ws = connected(&w).await;
    let pat = Pattern::new(14);
    let mut c = http(&w).await;
    let files = [
        ("a", "..\\..\\evil:name?.exe", 3),
        ("b", "CON.txt", 3),
        ("c", "", 3),
        ("d", "photo\u{202e}gpj.exe", 3),
        ("e", "trailing.dots...", 3),
        ("f", "dir/sub/file.txt", 3),
    ];
    assert_eq!(c.post_transfer("clean", &files).await.status, 200);
    for (id, _, size) in files {
        assert_eq!(c.put("clean", id, 0, size, &pat, None).await.unwrap().0.status, 200);
    }
    assert_eq!(listing(&w.core().received_dir()), ["_CON.txt", "evil_name_.exe", "file", "file.txt", "photo_gpj.exe", "trailing.dots"]);
}

#[tokio::test]
async fn the_control_channel_stays_quick_while_a_big_file_is_uploaded() {
    let w = World::new().await;
    let mut ws = connected(&w).await;
    let pat = Pattern::new(15);
    let size = 300 * MIB;
    let mut c = http(&w).await;
    assert_eq!(c.post_transfer("busy", &[("f", "big.bin", size)]).await.status, 200);
    let upload = tokio::spawn(async move { c.put("busy", "f", 0, size, &pat, None).await.map(|(r, d)| (r.status, d)) });
    // Pings from the PC are acked by the phone while the bytes flow; the round trip stays small.
    let core = w.env.core.clone();
    wait_until("the upload to start", || core.transfers.get("busy").is_some_and(|e| e.files[0].done() > 10 * MIB)).await;
    let mut worst = Duration::ZERO;
    for _ in 0..5 {
        let core = w.env.core.clone();
        let ping = tokio::spawn(async move { core.ping_phone().await });
        let msg = next_json(&mut ws).await;
        assert_eq!(msg["type"], "ping");
        send_json(&mut ws, &json!({ "type": "ack", "ref": msg["id"] })).await;
        match ping.await.unwrap() {
            kin_lib::core::PingReport::Rtt(d) => worst = worst.max(d),
            other => panic!("the ping failed during the upload: {other:?}"),
        }
    }
    let (status, elapsed) = upload.await.unwrap().unwrap();
    assert_eq!(status, 200);
    eprintln!("300 MiB upload with pings: {:.0} MB/s, slowest ping round trip {worst:?}", mb_per_s(size, elapsed));
    assert!(worst < Duration::from_millis(500), "a ping took {worst:?} during the upload");
}

// ----- what a restart and the clock do -------------------------------------------------------------------------------------

/// A core with a database file in `data` and received files in `recv`, as the app has it. (The in-memory database of the other tests
/// can't be reopened.)
fn core_at(data: &Path, recv: &Path) -> std::sync::Arc<Core> {
    use kin_lib::cli::Config;
    use kin_lib::clipboard::{MemoryClipboard, MonotonicClock};
    use kin_lib::core::Services;
    use kin_lib::links::RecordingOpener;
    std::fs::create_dir_all(data).unwrap();
    let key = &vector_keys()[0];
    let mut config = Config::new(data.to_path_buf());
    config.dev_bind = Some("127.0.0.1".parse().unwrap());
    config.dev_received_dir = Some(recv.to_path_buf());
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let services = Services {
        clipboard: std::sync::Arc::new(MemoryClipboard::default()),
        opener: std::sync::Arc::new(RecordingOpener::default()),
        clock: std::sync::Arc::new(MonotonicClock::new()),
    };
    std::sync::Arc::new(Core::new(
        config,
        identity_of(key),
        kin_lib::storage::Db::open(&data.join("kin.db")).unwrap(),
        "Desk PC".into(),
        tx,
        services,
    ))
}

fn put_params(offset: u64, length: u64) -> kin_lib::transfers::PutParams {
    kin_lib::transfers::PutParams { offset: Some(offset.to_string()), content_length: Some(length) }
}

fn body_of(pat: &Pattern, from: u64, to: u64, fail_at_end: bool) -> axum::body::Body {
    let mut items: Vec<Result<axum::body::Bytes, std::io::Error>> = Vec::new();
    let mut pos = from;
    while pos < to {
        let n = (to - pos).min(65536) as usize;
        items.push(Ok(axum::body::Bytes::from(pat.bytes(pos, n))));
        pos += n as u64;
    }
    if fail_at_end {
        items.push(Err(std::io::Error::other("the connection broke")));
    }
    axum::body::Body::from_stream(futures_util::stream::iter(items))
}

fn spec(id: &str, name: &str, size: u64) -> kin_lib::transfers::FileSpec {
    kin_lib::transfers::FileSpec { file_id: id.into(), name: name.into(), size, mime: "application/octet-stream".into() }
}

#[tokio::test]
async fn after_a_restart_the_transfer_is_there_with_what_was_stored() {
    let root = tempfile::tempdir().unwrap();
    let (data, recv) = (root.path().join("data"), root.path().join("recv"));
    let pat = Pattern::new(16);
    let size = 5 * MIB;
    let request = kin_lib::transfers::UploadRequest { id: "restart".into(), files: vec![spec("f", "movie.mkv", size)] };
    {
        let core = core_at(&data, &recv);
        core.upload_post("phone", &request).await.unwrap();
        let broken = core.upload_put("phone", "restart", "f", put_params(0, size), body_of(&pat, 0, 2 * MIB + 300, true)).await;
        assert!(broken.is_err(), "the body broke");
        assert_eq!(core.transfers.get("restart").unwrap().files[0].done(), 2 * MIB + 300);
    }
    // The app starts again: a new core on the same database.
    let core = core_at(&data, &recv);
    let entry = core.transfers.get("restart").expect("the transfer was loaded");
    assert_eq!((entry.files[0].done(), entry.status()), (2 * MIB + 300, Status::Active));
    let offsets = core.upload_post("phone", &request).await.unwrap();
    assert_eq!(offsets, [("f".to_string(), 2 * MIB + 300)]);
    core.upload_put("phone", "restart", "f", put_params(2 * MIB + 300, size - (2 * MIB + 300)), body_of(&pat, 2 * MIB + 300, size, false))
        .await
        .unwrap();
    pat.check_file(&recv.join("movie.mkv"), size).unwrap();
    // A third start: the finished transfer is still known for a day, so a repeated POST is answered.
    let core = core_at(&data, &recv);
    assert_eq!(core.upload_post("phone", &request).await.unwrap(), [("f".to_string(), size)]);
}

#[tokio::test]
async fn the_disk_is_the_check_after_a_restart_a_part_file_that_is_short_or_long_or_gone() {
    let root = tempfile::tempdir().unwrap();
    let (data, recv) = (root.path().join("data"), root.path().join("recv"));
    let pat = Pattern::new(17);
    let size = 3 * MIB;
    let request = kin_lib::transfers::UploadRequest {
        id: "disk".into(),
        files: vec![spec("f", "a.bin", size), spec("g", "b.bin", size), spec("h", "c.bin", size)],
    };
    {
        let core = core_at(&data, &recv);
        core.upload_post("phone", &request).await.unwrap();
        for f in ["f", "g", "h"] {
            let _ = core.upload_put("phone", "disk", f, put_params(0, size), body_of(&pat, 0, MIB, true)).await;
        }
    }
    // While the app was not running: one .part was cut short, one grew (a crash after the write and before the database), one was deleted.
    let part = |n: &str| recv.join(format!("{n}.part"));
    std::fs::OpenOptions::new().write(true).open(part("a.bin")).unwrap().set_len(MIB / 2).unwrap();
    std::fs::OpenOptions::new().write(true).open(part("b.bin")).unwrap().set_len(MIB + 5000).unwrap();
    std::fs::remove_file(part("c.bin")).unwrap();
    let core = core_at(&data, &recv);
    let got = core.upload_post("phone", &request).await.unwrap();
    assert_eq!(got, [("f".to_string(), MIB / 2), ("g".to_string(), MIB), ("h".to_string(), 0)], "the smaller of the database and the disk");
    // The one that grew is cut back to what the database said, and then completes correctly.
    core.upload_put("phone", "disk", "g", put_params(MIB, size - MIB), body_of(&pat, MIB, size, false)).await.unwrap();
    pat.check_file(&recv.join("b.bin"), size).unwrap();
    core.upload_put("phone", "disk", "f", put_params(MIB / 2, size - MIB / 2), body_of(&pat, MIB / 2, size, false)).await.unwrap();
    pat.check_file(&recv.join("a.bin"), size).unwrap();
    // The deleted one starts again from nothing, under the same name.
    core.upload_put("phone", "disk", "h", put_params(0, size), body_of(&pat, 0, size, false)).await.unwrap();
    pat.check_file(&recv.join("c.bin"), size).unwrap();
}

#[tokio::test]
async fn part_files_are_kept_a_day_and_then_cleaned_up() {
    let root = tempfile::tempdir().unwrap();
    let (data, recv) = (root.path().join("data"), root.path().join("recv"));
    let pat = Pattern::new(18);
    let core = core_at(&data, &recv);
    let request = kin_lib::transfers::UploadRequest { id: "old".into(), files: vec![spec("f", "a.bin", 5_000)] };
    core.upload_post("phone", &request).await.unwrap();
    let _ = core.upload_put("phone", "old", "f", put_params(0, 5_000), body_of(&pat, 0, 2_000, true)).await;
    // A .part file that no record names (a download of another program, say), and a file of the user's that isn't one.
    std::fs::write(recv.join("orphan.zip.part"), "x").unwrap();
    std::fs::write(recv.join("keep.txt"), "mine").unwrap();
    // A finished file that is *named* like a .part file is the user's, however old: a transfer recorded it.
    let pat2 = Pattern::new(19);
    let request2 = kin_lib::transfers::UploadRequest { id: "named".into(), files: vec![spec("f", "backup.part", 100)] };
    core.upload_post("phone", &request2).await.unwrap();
    core.upload_put("phone", "named", "f", put_params(0, 100), body_of(&pat2, 0, 100, false)).await.unwrap();
    assert!(recv.join("backup.part").exists());
    // Now: nothing is a day old, so nothing goes.
    core.clean_up().await;
    assert_eq!(listing(&recv), ["a.bin.part", "backup.part", "keep.txt", "orphan.zip.part"]);
    // The unrecorded file gets a day older, and the transfer has had no progress for a day.
    let old = std::time::SystemTime::now() - Duration::from_secs(25 * 3600);
    // (the toast and the picture search of a finished upload open the file for a moment, in a task of their own: try again if it is busy)
    for name in ["orphan.zip.part", "backup.part"] {
        let mut tries = 0;
        loop {
            match std::fs::OpenOptions::new().write(true).open(recv.join(name)) {
                Ok(f) => break f.set_modified(old).unwrap(),
                Err(e) if tries < 100 && e.raw_os_error() == Some(32) => {
                    tries += 1;
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => panic!("{name}: {e}"),
            }
        }
    }
    core.db
        .touch_transfer(
            "old",
            (std::time::SystemTime::now() - Duration::from_secs(25 * 3600)).duration_since(std::time::UNIX_EPOCH).unwrap().as_millis()
                as i64,
        )
        .unwrap();
    core.clean_up().await;
    assert_eq!(
        listing(&recv),
        ["backup.part", "keep.txt", "orphan.zip.part"],
        "the old transfer's .part file is gone; a .part file that no record names is the user's, however old, and stays"
    );
    assert!(core.transfers.get("old").is_none());
    assert_eq!(core.db.get_transfer("old").unwrap().unwrap().0.status, "failed");
}

// ----- received files and the clipboard, and what a click may open --------------------------------------------------------

async fn receive(w: &World, id: &str, files: &[(&str, &str, u64)], mime: &str) {
    let pat = Pattern::new(99);
    let mut c = http(w).await;
    let body: Vec<Value> = files.iter().map(|(f, n, s)| json!({ "fileId": f, "name": n, "size": s, "mime": mime })).collect();
    assert_eq!(c.json("POST", "/v1/transfers", &json!({ "id": id, "files": body })).await.status, 200);
    for (f, _, s) in files {
        assert_eq!(c.put(id, f, 0, *s, &pat, None).await.unwrap().0.status, 200);
    }
}

/// Uploads files with these bytes (one `POST`, one `PUT` each).
async fn receive_bytes(w: &World, id: &str, files: &[(&str, &str, Vec<u8>)]) {
    let mut c = http(w).await;
    let body: Vec<Value> =
        files.iter().map(|(f, n, b)| json!({ "fileId": f, "name": n, "size": b.len(), "mime": "application/octet-stream" })).collect();
    assert_eq!(c.json("POST", "/v1/transfers", &json!({ "id": id, "files": body })).await.status, 200);
    for (f, _, bytes) in files {
        assert_eq!(c.put_bytes(id, f, bytes).await.unwrap().status, 200);
    }
}

#[tokio::test]
async fn one_received_image_also_goes_to_the_clipboard_and_other_files_and_several_images_do_not() {
    let w = World::new().await;
    let _ws = connected(&w).await;
    let dir = w.core().received_dir();
    let text = b"just text".to_vec();
    receive_bytes(&w, "i1", &[("f", "photo.jpg", TINY_PNG.to_vec())]).await;
    wait_until("the image on the clipboard", || w.env.clipboard.app_images() == [dir.join("photo.jpg")]).await;
    receive_bytes(&w, "i2", &[("f", "notes.txt", text.clone())]).await;
    receive_bytes(&w, "i3", &[("f", "a.png", TINY_PNG.to_vec()), ("g", "b.png", TINY_PNG.to_vec())]).await;
    receive_bytes(&w, "i4", &[("f", "c.png", TINY_PNG.to_vec()), ("g", "doc.pdf", text)]).await;
    wait_until("the photo of the mixed transfer", || w.env.clipboard.app_images().len() == 2).await;
    // Give the transfers that must not touch the clipboard time to (wrongly) do it.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        w.env.clipboard.app_images(),
        [dir.join("photo.jpg"), dir.join("c.png")],
        "a photo with a document: the photo; two photos: none; text: none"
    );
}

#[tokio::test]
async fn what_is_an_image_is_decided_by_its_content_not_by_the_name_or_the_type_the_phone_gave() {
    let w = World::new().await;
    let _ws = connected(&w).await;
    let dir = w.core().received_dir();
    // Text with an image name and an image type: not an image. A real picture with a plain name and a wrong type: an image.
    let mut c = http(&w).await;
    let body = json!({ "id": "c1", "files": [{ "fileId": "f", "name": "fake.png", "size": 9, "mime": "image/png" }] });
    assert_eq!(c.json("POST", "/v1/transfers", &body).await.status, 200);
    assert_eq!(c.put_bytes("c1", "f", b"not a png").await.unwrap().status, 200);
    let mut c = http(&w).await;
    let body =
        json!({ "id": "c2", "files": [{ "fileId": "f", "name": "scan.dat", "size": TINY_PNG.len(), "mime": "application/octet-stream" }] });
    assert_eq!(c.json("POST", "/v1/transfers", &body).await.status, 200);
    assert_eq!(c.put_bytes("c2", "f", TINY_PNG).await.unwrap().status, 200);
    wait_until("the real picture on the clipboard", || w.env.clipboard.app_images() == [dir.join("scan.dat")]).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(w.env.clipboard.app_images(), [dir.join("scan.dat")], "the text named .png never went there");
}

#[tokio::test]
async fn images_to_the_clipboard_can_be_turned_off() {
    let mut w = World::new().await;
    let _ws = connected(&w).await;
    w.core().set_images_to_clipboard(false);
    let saved = kin_lib::settings::Settings::load(&w.core().config.data_dir);
    assert!(!saved.images_to_clipboard, "the choice is kept in settings.json");
    receive_bytes(&w, "off", &[("f", "photo.jpg", TINY_PNG.to_vec())]).await;
    wait_toasts(&mut w).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(w.env.clipboard.app_images().is_empty());
}

#[tokio::test]
async fn a_click_never_runs_a_received_program() {
    let mut w = World::new().await;
    let _ws = connected(&w).await;
    drain_events(&mut w);
    let dir = w.core().received_dir();
    // Programs, scripts that other programs on the PC register for themselves, and a double extension: never opened by a click.
    for (i, name) in ["setup.exe", "run.sh", "tool.pyz", "evil.pdf.exe"].into_iter().enumerate() {
        receive(&w, &format!("x{i}"), &[("f", name, 100)], "application/octet-stream").await;
        let toasts = wait_toasts(&mut w).await;
        assert_eq!(toasts[0].1, "1 file from Test Phone");
        assert!(w.core().toast_clicked(toasts[0].0));
        assert!(w.env.opener.opened_files().is_empty(), "{name} is never opened by a click");
        assert_eq!(w.env.opener.shown_in_folder().last(), Some(&dir.join(name)), "{name} is shown in its folder instead");
    }
    // A type on the allow-list opens.
    receive(&w, "ok", &[("f", "report.pdf", 100)], "application/pdf").await;
    let toasts = wait_toasts(&mut w).await;
    assert!(w.core().toast_clicked(toasts[0].0));
    assert_eq!(w.env.opener.opened_files(), [dir.join("report.pdf")]);
}

#[tokio::test]
async fn a_toast_click_that_matches_no_toast_opens_nothing() {
    let w = World::new().await;
    let _ws = connected(&w).await;
    receive(&w, "y1", &[("f", "a.txt", 10)], "text/plain").await;
    assert!(!w.core().toast_clicked(424_242));
    assert!(w.env.opener.opened_files().is_empty() && w.env.opener.opened().is_empty() && w.env.opener.opened_folders().is_empty());
}

// ----- a 2 GB file ---------------------------------------------------------------------------------------------------------

/// Run with `cargo test --release large_upload -- --ignored --nocapture`. The file is made by the test (a pattern, never held in
/// memory) and lands in a temporary folder that is deleted afterwards.
#[tokio::test]
#[ignore = "moves 2 GB: run with --ignored --nocapture in release mode"]
async fn large_upload_of_2_gb() {
    let w = World::new().await;
    let _ws = connected(&w).await;
    let pat = Pattern::new(20);
    let size = 2 * 1024 * MIB;
    let mut c = http(&w).await;
    assert_eq!(c.post_transfer("huge", &[("f", "huge.bin", size)]).await.status, 200);
    let (r, elapsed) = c.put("huge", "f", 0, size, &pat, None).await.unwrap();
    assert_eq!(r.status, 200);
    println!(
        "THROUGHPUT upload 2 GB (phone -> PC, loopback TLS, written to disk): {:.0} MB/s in {:.1} s",
        mb_per_s(size, elapsed),
        elapsed.as_secs_f64()
    );
    let checked = std::time::Instant::now();
    pat.check_file(&w.core().received_dir().join("huge.bin"), size).unwrap();
    println!("verified 2 GB on disk in {:.1} s", checked.elapsed().as_secs_f64());
}

/// A 2 GB upload that breaks in the middle and resumes (the Wi-Fi goes off and on).
#[tokio::test]
#[ignore = "moves 2 GB: run with --ignored --nocapture in release mode"]
async fn large_upload_of_2_gb_that_breaks_and_resumes() {
    let w = World::new().await;
    let _ws = connected(&w).await;
    let pat = Pattern::new(21);
    let size = 2 * 1024 * MIB;
    let mut c = http(&w).await;
    assert_eq!(c.post_transfer("huge2", &[("f", "huge2.bin", size)]).await.status, 200);
    assert!(c.put("huge2", "f", 0, size, &pat, Some(1_000 * MIB + 4_321)).await.is_none());
    drop(c);
    let mut c = http(&w).await;
    let r = c.post_transfer("huge2", &[("f", "huge2.bin", size)]).await;
    let offset = offsets(&r)[0].1;
    println!("the PC kept {offset} bytes");
    assert_eq!(offset, 1_000 * MIB + 4_321);
    let (r, elapsed) = c.put("huge2", "f", offset, size, &pat, None).await.unwrap();
    assert_eq!(r.status, 200);
    println!("THROUGHPUT resumed upload ({} bytes): {:.0} MB/s", size - offset, mb_per_s(size - offset, elapsed));
    pat.check_file(&w.core().received_dir().join("huge2.bin"), size).unwrap();
}
