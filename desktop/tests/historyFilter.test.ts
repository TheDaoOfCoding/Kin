import { describe, expect, test } from "bun:test";
import {
  DIRECTION_OPTIONS,
  type FilterItem,
  type Query,
  SORT_OPTIONS,
  TYPE_OPTIONS,
  type TypeId,
  applyQuery,
  chips,
  defaultQuery,
  isDefault,
  toggleType,
  without,
} from "../src/historyFilter";

type Item = FilterItem & { id: string; status?: string };

function file(id: string, names: string[], types: string[], o: Partial<Item> = {}): Item {
  return { id, direction: "in", kind: "file", createdMs: 1, totalBytes: 100, names, text: null, isUrl: false, isLink: false, title: null, types, ...o };
}
function text(id: string, body: string, o: Partial<Item> = {}): Item {
  return { id, direction: "out", kind: "text", createdMs: 500, totalBytes: body.length, names: [], text: body, isUrl: false, isLink: false, title: null, types: ["text"], ...o };
}
function link(id: string, url: string, title: string | null, o: Partial<Item> = {}): Item {
  return text(id, url, { isUrl: true, isLink: true, title, types: ["link"], ...o });
}

const q = (o: Partial<Query>): Query => ({ ...defaultQuery(), ...o });
const ids = (items: Item[]) => items.map((i) => i.id);

// Newest first, as the backend sends them.
const photo = file("photo", ["IMG_1.JPG"], ["photo"], { createdMs: 900, totalBytes: 4000, direction: "in" });
const video = file("video", ["clip.MOV"], ["video"], { createdMs: 800, totalBytes: 90000, direction: "out" });
const pdf = file("pdf", ["Report.pdf"], ["document"], { createdMs: 700, totalBytes: 2000, direction: "out" });
const apk = file("apk", ["kin.apk"], ["app"], { createdMs: 600, totalBytes: 30000, direction: "in" });
const zip = file("zip", ["backup.zip"], ["other"], { createdMs: 500, totalBytes: 500, direction: "in" });
const mixed = file("mixed", ["a.jpg", "b.pdf"], ["document", "photo"], { createdMs: 400, totalBytes: 7000, direction: "out" });
const note = text("note", "Buy milk", { createdMs: 300 });
const site = link("site", "https://example.com/x", "Example Domain", { createdMs: 200, direction: "in" });
const bare = link("bare", "https://zeta.test", null, { createdMs: 100 });
const all = [photo, video, pdf, apk, zip, mixed, note, site, bare];

describe("type filter", () => {
  test("each type shows its items", () => {
    const want: Record<TypeId, string[]> = {
      photo: ["photo", "mixed"],
      video: ["video"],
      document: ["pdf", "mixed"],
      app: ["apk"],
      other: ["zip"],
      text: ["note"],
      link: ["site", "bare"],
    };
    for (const o of TYPE_OPTIONS) expect(ids(applyQuery(all, q({ types: [o.id] })))).toEqual(want[o.id]);
  });

  test("an item matches when any of its files is of the type", () => {
    expect(ids(applyQuery([mixed], q({ types: ["photo"] })))).toEqual(["mixed"]);
    expect(ids(applyQuery([mixed], q({ types: ["document"] })))).toEqual(["mixed"]);
    expect(ids(applyQuery([mixed], q({ types: ["video"] })))).toEqual([]);
  });

  test("several types show an item matching any of them", () => {
    expect(ids(applyQuery(all, q({ types: ["video", "app", "link"] })))).toEqual(["video", "apk", "site", "bare"]);
  });

  test("no type selected shows everything", () => {
    expect(ids(applyQuery(all, defaultQuery()))).toEqual(ids(all));
  });

  test("an item without types is hidden by a type filter and kept without one", () => {
    const old = { ...photo, types: undefined } as unknown as Item;
    expect(applyQuery([old], q({ types: ["photo"] }))).toEqual([]);
    expect(applyQuery([old], defaultQuery())).toEqual([old]);
  });
});

describe("groups combine with AND", () => {
  test("type and direction", () => {
    expect(ids(applyQuery(all, q({ types: ["photo", "document"], direction: "out" })))).toEqual(["pdf", "mixed"]);
  });
  test("type, direction and search", () => {
    expect(ids(applyQuery(all, q({ types: ["photo", "document"], direction: "out", search: "b.pdf" })))).toEqual(["mixed"]);
  });
  test("a search nothing matches gives an empty list", () => {
    expect(applyQuery(all, q({ search: "zzz" }))).toEqual([]);
  });
});

describe("direction", () => {
  test("Sent, Received and Both", () => {
    expect(ids(applyQuery(all, q({ direction: "in" })))).toEqual(["photo", "apk", "zip", "site"]);
    expect(ids(applyQuery(all, q({ direction: "out" })))).toEqual(["video", "pdf", "mixed", "note", "bare"]);
    expect(applyQuery(all, q({ direction: "both" }))).toHaveLength(all.length);
  });
  test("failed and waiting items count in their direction", () => {
    const failed = file("failed", ["x.png"], ["photo"], { createdMs: 50, direction: "out", status: "failed" });
    const waiting = file("waiting", ["y.png"], ["photo"], { createdMs: 40, direction: "in", status: "queued" });
    expect(ids(applyQuery([failed, waiting], q({ direction: "out" })))).toEqual(["failed"]);
    expect(ids(applyQuery([failed, waiting], q({ direction: "in" })))).toEqual(["waiting"]);
  });
});

describe("search", () => {
  test("file names, any case", () => {
    expect(ids(applyQuery(all, q({ search: "img_1" })))).toEqual(["photo"]);
    expect(ids(applyQuery(all, q({ search: "REPORT" })))).toEqual(["pdf"]);
  });
  test("a name beyond the first one is found too", () => {
    expect(ids(applyQuery(all, q({ search: "b.pdf" })))).toEqual(["mixed"]);
  });
  test("the text preview", () => {
    expect(ids(applyQuery(all, q({ search: "milk" })))).toEqual(["note"]);
  });
  test("the title and the address of a link", () => {
    expect(ids(applyQuery(all, q({ search: "domain" })))).toEqual(["site"]);
    expect(ids(applyQuery(all, q({ search: "zeta.test" })))).toEqual(["bare"]);
  });
  test("it is a contains, and surrounding spaces do not count", () => {
    expect(ids(applyQuery(all, q({ search: "  mil " })))).toEqual(["note"]);
    expect(applyQuery(all, q({ search: "   " }))).toHaveLength(all.length);
  });
});

describe("sort", () => {
  test("Newest keeps newest first and puts a newer item before an older one", () => {
    expect(ids(applyQuery([zip, photo, note], q({ sort: "newest" })))).toEqual(["photo", "zip", "note"]);
  });
  test("Newest keeps the order of equal times", () => {
    const a = text("a", "a", { createdMs: 5 });
    const b = text("b", "b", { createdMs: 5 });
    expect(ids(applyQuery([a, b], defaultQuery()))).toEqual(["a", "b"]);
  });
  test("Oldest is the reverse", () => {
    expect(ids(applyQuery(all, q({ sort: "oldest" })))).toEqual([...ids(all)].reverse());
  });
  test("Name goes by the first file name, a link's title else its address, a text's words", () => {
    // a.jpg, backup.zip, Buy milk, clip.MOV, Example Domain, https://zeta.test, IMG_1.JPG, kin.apk, Report.pdf
    expect(ids(applyQuery(all, q({ sort: "name" })))).toEqual(["mixed", "zip", "note", "video", "site", "bare", "photo", "apk", "pdf"]);
  });
  test("Name ignores case and ties go newest first", () => {
    const upper = file("upper", ["Same.TXT"], ["document"], { createdMs: 10 });
    const lower = file("lower", ["same.txt"], ["document"], { createdMs: 20 });
    expect(ids(applyQuery([upper, lower], q({ sort: "name" })))).toEqual(["lower", "upper"]);
  });
  test("Size is largest first, with texts and links after all the files", () => {
    expect(ids(applyQuery(all, q({ sort: "size" })))).toEqual(["video", "apk", "mixed", "photo", "pdf", "zip", "note", "site", "bare"]);
  });
  test("Size: equal sizes and the texts go newest first", () => {
    const small1 = file("s1", ["a.bin"], ["other"], { createdMs: 10, totalBytes: 5 });
    const small2 = file("s2", ["b.bin"], ["other"], { createdMs: 20, totalBytes: 5 });
    const bigText = text("bigtext", "x".repeat(10_000), { createdMs: 1, totalBytes: 10_000 });
    const newText = text("newtext", "y", { createdMs: 99, totalBytes: 1 });
    expect(ids(applyQuery([small1, bigText, small2, newText], q({ sort: "size" })))).toEqual(["s2", "s1", "newtext", "bigtext"]);
  });
  test("the input list is not changed", () => {
    const copy = [...all];
    applyQuery(all, q({ sort: "name" }));
    expect(all).toEqual(copy);
  });
});

describe("chips", () => {
  test("none for the default query", () => {
    expect(chips(defaultQuery())).toEqual([]);
    expect(isDefault(defaultQuery())).toBe(true);
  });
  test("one per choice, with short labels", () => {
    const all3 = chips(q({ types: ["video", "photo"], direction: "out", sort: "oldest", search: " cat " }));
    expect(all3.map((c) => c.label)).toEqual(["Photos", "Videos", "Sent", "Oldest", '"cat"']);
    expect(chips(q({ direction: "in" }))[0].label).toBe("Received");
    expect(chips(q({ sort: "name" }))[0].label).toBe("Name");
    expect(chips(q({ sort: "size" }))[0].label).toBe("Size");
  });
  test("a blank search has no chip", () => {
    expect(chips(q({ search: "   " }))).toEqual([]);
  });
  test("the x of a chip resets that one choice", () => {
    const full = q({ types: ["video", "photo"], direction: "out", sort: "oldest", search: "cat" });
    expect(without(full, "type:photo").types).toEqual(["video"]);
    expect(without(full, "direction").direction).toBe("both");
    expect(without(full, "sort").sort).toBe("newest");
    expect(without(full, "search").search).toBe("");
    expect(without(without(without(without(without(full, "type:photo"), "type:video"), "direction"), "sort"), "search")).toEqual(defaultQuery());
    expect(without(full, "nope")).toEqual(full);
  });
  test("toggleType switches a type on and off without changing the query it got", () => {
    const base = defaultQuery();
    const on = toggleType(base, "app");
    expect(on.types).toEqual(["app"]);
    expect(base.types).toEqual([]);
    expect(toggleType(on, "app").types).toEqual([]);
  });
});

describe("words", () => {
  test("no em dash in a label or a chip", () => {
    const strings = [
      ...TYPE_OPTIONS.map((o) => o.label),
      ...SORT_OPTIONS.map((o) => o.label),
      ...DIRECTION_OPTIONS.map((o) => o.label),
      ...chips(q({ types: TYPE_OPTIONS.map((o) => o.id), direction: "out", sort: "size", search: "a b" })).map((c) => c.label),
    ];
    expect(strings.length).toBeGreaterThan(10);
    for (const s of strings) expect(s).not.toContain("—");
  });
  test("the labels are the ones of the plan", () => {
    expect(TYPE_OPTIONS.map((o) => o.label)).toEqual(["Photos", "Videos", "Documents", "Apps", "Other files", "Texts", "Links"]);
    expect(SORT_OPTIONS.map((o) => o.label)).toEqual(["Newest", "Oldest", "Name", "Size"]);
    expect(DIRECTION_OPTIONS.map((o) => o.label)).toEqual(["Both", "Sent", "Received"]);
  });
});
