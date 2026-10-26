// Icons, the logo and the pictures of the new look. The sprite (src/sprite.ts) is put in each page once; everything here only points into it
// with `<use href="#...">`. The pictures are made for both skins: Warm shows the illustration, Simple the plain icon (the CSS decides which, by
// the skin on the page: `.only-ill` and `.only-plain`). The markup is part of this program, never data.
import { SPRITE } from "./sprite";

const SVG_NS = "http://www.w3.org/2000/svg";

function svgFrom(markup: string): SVGElement {
  const doc = new DOMParser().parseFromString(markup.includes("xmlns=") ? markup : markup.replace("<svg", `<svg xmlns="${SVG_NS}"`), "image/svg+xml");
  return document.importNode(doc.documentElement, true) as unknown as SVGElement;
}

/** Puts the sprite in the page. Call once, before anything is drawn. */
export function installSprite(): void {
  if (!document.getElementById("kin-sprite")) {
    const sprite = svgFrom(SPRITE);
    sprite.id = "kin-sprite";
    document.body.prepend(sprite);
  }
}

/** A 24 px icon of the sprite ("home", "send", ...), as large as `size` says (a class of styles.css: "s16", "s18", "s20", "s28") or the container makes it. */
export function icon(name: string, className = ""): SVGElement {
  return svgFrom(`<svg class="ic ${className}" viewBox="0 0 24 24" aria-hidden="true"><use href="#i-${name}"/></svg>`);
}

/** The logo: the two friends of the Kin mark on apricot, the same in every skin and mode. */
export function logo(className = ""): HTMLElement {
  const box = document.createElement("span");
  box.className = `logo ${className}`;
  box.append(svgFrom(`<svg viewBox="0 0 40 40" aria-hidden="true"><use href="#kin-mark"/></svg>`));
  return box;
}

/** A box of the page that holds a picture: the illustration in Warm, the plain icon in Simple. */
function art(className: string, illustration: string, plain: string): HTMLElement {
  const box = document.createElement("span");
  box.className = `art ${className}`;
  box.setAttribute("aria-hidden", "true");
  box.append(svgFrom(illustration), svgFrom(plain));
  return box;
}

const use = (id: string, extra = "") => `<use href="#${id}" width="146" height="104" ${extra}/>`;

/** The phone on the home page. Its face follows `data-state` of the page: connected, connecting (looking for it) or paused. */
export function phoneArt(className = "phero-art"): HTMLElement {
  return art(
    className,
    `<svg class="only-ill" viewBox="98 34 44 66"><g class="bob">${use("il-p")}<use data-show="connected" href="#il-p-happy" width="146" height="104"/><use data-show="connecting" href="#il-p-look" width="146" height="104"/><use data-show="paused" href="#il-p-sleep" width="146" height="104"/></g></svg>`,
    `<svg class="only-plain" viewBox="0 0 64 92"><use class="pl-ic" href="#i-phone" x="8" y="22" width="48" height="48" style="stroke-width:1.1"/><circle class="pl-dot" cx="44" cy="26" r="6"/></svg>`,
  );
}

/** The happy box of the drop zone. */
export function boxArt(className = "drop-art"): HTMLElement {
  return art(
    className,
    `<svg class="only-ill" viewBox="0 0 170 128"><use href="#il-box" width="170" height="128"/></svg>`,
    `<svg class="only-plain" viewBox="0 0 170 128"><use class="pl-ic" href="#i-upload" x="57" y="36" width="56" height="56" style="stroke-width:1"/></svg>`,
  );
}

/** The picture of an empty list: the box in Warm, the icon `name` in Simple. */
export function emptyArt(name: string): HTMLElement {
  return art(
    "empty-art",
    `<svg class="only-ill" viewBox="0 0 170 128"><use href="#il-box" width="170" height="128"/></svg>`,
    `<svg class="only-plain" viewBox="0 0 170 128"><use class="pl-ic" href="#i-${name}" x="57" y="36" width="56" height="56" style="stroke-width:1"/></svg>`,
  );
}

/** The PC and the phone, side by side and pleased, for the pairing window. */
export function pairArt(className = "wl-art"): HTMLElement {
  return art(
    className,
    `<svg class="only-ill" viewBox="0 0 146 104">${use("il-t")}${use("il-t-happy")}<g class="bob">${use("il-p")}${use("il-p-happy")}</g></svg>`,
    `<svg class="only-plain" viewBox="0 0 146 104">${use("pl-pair")}</svg>`,
  );
}

/** The cover of the media card when the phone sent no picture: a record in Warm, a note in Simple. */
export function mediaArt(): HTMLElement {
  return art(
    "media-art",
    `<svg class="only-ill" viewBox="0 0 50 50"><use href="#il-vinyl" width="50" height="50"/></svg>`,
    `<svg class="only-plain" viewBox="0 0 50 50"><use class="pl-ic" href="#i-music" x="9" y="9" width="32" height="32" style="stroke-width:1.5"/></svg>`,
  );
}
