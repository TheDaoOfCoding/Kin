import { describe, expect, test } from "bun:test";
import { shouldStamp, usableTime, withTimestamp, youtubeVideoId } from "../../src/shared/youtube";

describe("youtubeVideoId", () => {
  test("watch pages on every YouTube host", () => {
    for (const host of ["www.youtube.com", "youtube.com", "m.youtube.com", "music.youtube.com"]) {
      expect(youtubeVideoId(`https://${host}/watch?v=dQw4w9WgXcQ`)).toBe("dQw4w9WgXcQ");
    }
  });
  test("youtu.be and /live/", () => {
    expect(youtubeVideoId("https://youtu.be/dQw4w9WgXcQ?si=abc")).toBe("dQw4w9WgXcQ");
    expect(youtubeVideoId("https://www.youtube.com/live/dQw4w9WgXcQ")).toBe("dQw4w9WgXcQ");
  });
  test("not video pages", () => {
    expect(youtubeVideoId("https://www.youtube.com/")).toBeNull();
    expect(youtubeVideoId("https://www.youtube.com/@channel")).toBeNull();
    expect(youtubeVideoId("https://www.youtube.com/shorts/dQw4w9WgXcQ")).toBeNull();
    expect(youtubeVideoId("https://www.youtube.com/watch")).toBeNull();
    expect(youtubeVideoId("https://www.youtube.com/watch?v=")).toBeNull();
  });
  test("lookalike hosts", () => {
    expect(youtubeVideoId("https://youtube.com.evil.example/watch?v=dQw4w9WgXcQ")).toBeNull();
    expect(youtubeVideoId("https://notyoutube.com/watch?v=dQw4w9WgXcQ")).toBeNull();
    expect(youtubeVideoId("https://example.com/watch?v=dQw4w9WgXcQ")).toBeNull();
    expect(youtubeVideoId("ftp://www.youtube.com/watch?v=dQw4w9WgXcQ")).toBeNull();
    expect(youtubeVideoId("nonsense")).toBeNull();
  });
});

describe("withTimestamp", () => {
  test("appends t=<n>s", () => {
    expect(withTimestamp("https://www.youtube.com/watch?v=abcdefghijk", 83)).toBe("https://www.youtube.com/watch?v=abcdefghijk&t=83s");
  });
  test("youtu.be with no query", () => {
    expect(withTimestamp("https://youtu.be/abcdefghijk", 5)).toBe("https://youtu.be/abcdefghijk?t=5s");
  });
  test("replaces an existing t and keeps the rest byte for byte", () => {
    expect(withTimestamp("https://www.youtube.com/watch?v=abcdefghijk&t=10&list=PL%20x+y", 61)).toBe(
      "https://www.youtube.com/watch?v=abcdefghijk&list=PL%20x+y&t=61s",
    );
    expect(withTimestamp("https://www.youtube.com/watch?t=1m2s&v=abcdefghijk", 7)).toBe("https://www.youtube.com/watch?v=abcdefghijk&t=7s");
  });
  test("keeps the fragment, drops fractions, never negative", () => {
    expect(withTimestamp("https://www.youtube.com/watch?v=abcdefghijk#x", 12.9)).toBe("https://www.youtube.com/watch?v=abcdefghijk&t=12s#x");
    expect(withTimestamp("https://youtu.be/abcdefghijk", -4)).toBe("https://youtu.be/abcdefghijk?t=0s");
  });
  test("does not touch a parameter that merely starts with t", () => {
    expect(withTimestamp("https://www.youtube.com/watch?v=abcdefghijk&tt=1", 3)).toBe("https://www.youtube.com/watch?v=abcdefghijk&tt=1&t=3s");
  });
  test("returns garbage unchanged", () => {
    expect(withTimestamp("nonsense", 3)).toBe("nonsense");
  });
});

describe("shouldStamp", () => {
  const tab = "https://www.youtube.com/watch?v=abcdefghijk&t=5s";
  test("the video on screen, in any link form", () => {
    expect(shouldStamp(tab, tab)).toBe(true);
    expect(shouldStamp("https://youtu.be/abcdefghijk", tab)).toBe(true);
  });
  test("a link to another video is left alone", () => {
    expect(shouldStamp("https://www.youtube.com/watch?v=zzzzzzzzzzz", tab)).toBe(false);
  });
  test("not on a YouTube tab, or the tab is unknown", () => {
    expect(shouldStamp(tab, "https://example.com/")).toBe(false);
    expect(shouldStamp(tab, undefined)).toBe(false);
  });
});

describe("usableTime", () => {
  test("a normal playing video", () => {
    expect(usableTime({ time: 83.7, seekable: true, ad: false })).toBe(83);
  });
  test("nothing worth writing", () => {
    expect(usableTime({ time: 0.4, seekable: true, ad: false })).toBeNull();
    expect(usableTime({ time: 100, seekable: false, ad: false })).toBeNull();
    expect(usableTime({ time: 100, seekable: true, ad: true })).toBeNull();
    expect(usableTime({ time: Number.NaN, seekable: true, ad: false })).toBeNull();
    expect(usableTime(null)).toBeNull();
  });
});
