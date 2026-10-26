// The icons and the pictures of the new look, as one SVG sprite that every window puts in its page (`installSprite`). Copied from the design mock
// (notes/ui-mocks/final/kin-warm-simple.html): the icons are a 24 px grid with a 2 px round stroke, `il-*` are the Warm illustrations (the PC and the
// phone with their faces, the box), `pl-pair` is the plain picture Simple shows in their place, `kin-mark` is the logo.
export const SPRITE = `<svg xmlns="http://www.w3.org/2000/svg" width="0" height="0" style="position:absolute" aria-hidden="true"><defs>
    <symbol id="i-send" viewBox="0 0 24 24"><path d="M20.5 3.5 10.2 13.8"/><path d="M20.5 3.5 14 20.5l-3.8-6.7-6.7-3.8 17-6.5z"/></symbol>
    <symbol id="i-phone" viewBox="0 0 24 24"><rect x="6.5" y="2.5" width="11" height="19" rx="3"/><path d="M10.5 18.5h3"/></symbol>
    <symbol id="i-bell" viewBox="0 0 24 24"><path d="M6 10.5a6 6 0 0 1 12 0v4.3l1.5 2.5a.6.6 0 0 1-.5.9H5a.6.6 0 0 1-.5-.9L6 14.8v-4.3z"/><path d="M10 21h4"/></symbol>
    <symbol id="i-ring" viewBox="0 0 24 24"><path d="M7 10.5a5 5 0 0 1 10 0v4l1.3 2.2a.5.5 0 0 1-.4.8H6.1a.5.5 0 0 1-.4-.8L7 14.5v-4z"/><path d="M10.3 20.5h3.4"/><path d="M3 9a9 9 0 0 1 2.4-4.6M21 9a9 9 0 0 0-2.4-4.6"/></symbol>
    <symbol id="i-pause" viewBox="0 0 24 24"><path d="M9 6v12M15 6v12"/></symbol>
    <symbol id="i-play" viewBox="0 0 24 24"><path d="M8 5.8v12.4a.6.6 0 0 0 .9.5l9.6-6.2a.6.6 0 0 0 0-1L8.9 5.3a.6.6 0 0 0-.9.5z"/></symbol>
    <symbol id="i-prev" viewBox="0 0 24 24"><path d="M18 6.6v10.8a.6.6 0 0 1-.9.5L9.4 12.5a.6.6 0 0 1 0-1l7.7-5.4a.6.6 0 0 1 .9.5z"/><path d="M6 6v12"/></symbol>
    <symbol id="i-next" viewBox="0 0 24 24"><path d="M6 6.6v10.8a.6.6 0 0 0 .9.5l7.7-5.4a.6.6 0 0 0 0-1L6.9 6.1a.6.6 0 0 0-.9.5z"/><path d="M18 6v12"/></symbol>
    <symbol id="i-music" viewBox="0 0 24 24"><path d="M9 17.5V5.8l10.5-2.1v11.8"/><circle cx="6.5" cy="17.5" r="2.5"/><circle cx="17" cy="15.5" r="2.5"/></symbol>
    <symbol id="i-chat" viewBox="0 0 24 24"><path d="M5.5 4.5h13a2.5 2.5 0 0 1 2.5 2.5v8a2.5 2.5 0 0 1-2.5 2.5H11l-4.5 3.5v-3.5h-1A2.5 2.5 0 0 1 3 15V7a2.5 2.5 0 0 1 2.5-2.5z"/><path d="M8 9.5h8M8 13h5"/></symbol>
    <symbol id="i-file" viewBox="0 0 24 24"><path d="M13.5 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8.5L13.5 3z"/><path d="M13.5 3v5.5H19"/></symbol>
    <symbol id="i-folder" viewBox="0 0 24 24"><path d="M3 7.5A2.5 2.5 0 0 1 5.5 5H9l2 2.5h7.5A2.5 2.5 0 0 1 21 10v7.5a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 3 17.5v-10z"/></symbol>
    <symbol id="i-check" viewBox="0 0 24 24"><path d="m5 12.5 4.5 4.5L19 7.5"/></symbol>
    <symbol id="i-alert" viewBox="0 0 24 24"><circle cx="12" cy="12" r="8.5"/><path d="M12 7.8v4.9"/><path d="M12 16.2h.01"/></symbol>
    <symbol id="i-info" viewBox="0 0 24 24"><circle cx="12" cy="12" r="8.5"/><path d="M12 11v5.2"/><path d="M12 7.8h.01"/></symbol>
    <symbol id="i-retry" viewBox="0 0 24 24"><path d="M20.5 12a8.5 8.5 0 1 1-2.6-6.1l2.6 2.6"/><path d="M20.5 4v4.5H16"/></symbol>
    <symbol id="i-home" viewBox="0 0 24 24"><path d="M3.5 10.2 12 3.5l8.5 6.7V19a1.5 1.5 0 0 1-1.5 1.5h-4v-6H9v6H5A1.5 1.5 0 0 1 3.5 19v-8.8z"/></symbol>
    <symbol id="i-history" viewBox="0 0 24 24"><path d="M3.5 12a8.5 8.5 0 1 0 2.6-6.1L3.5 8.5"/><path d="M3.5 4v4.5H8"/><path d="M12 7.5V12l3 2"/></symbol>
    <symbol id="i-settings" viewBox="0 0 24 24"><path d="M4 7h9M17 7h3M4 17h3M11 17h9"/><circle cx="15" cy="7" r="2"/><circle cx="9" cy="17" r="2"/></symbol>
    <symbol id="i-wifi" viewBox="0 0 24 24"><path d="M2.5 9.2a13.5 13.5 0 0 1 19 0"/><path d="M5.8 12.7a8.8 8.8 0 0 1 12.4 0"/><path d="M9 16.1a4.3 4.3 0 0 1 6 0"/><path d="M12 19.5h.01"/></symbol>
    <symbol id="i-chev-d" viewBox="0 0 24 24"><path d="m6 9.5 6 6 6-6"/></symbol>
    <symbol id="i-up" viewBox="0 0 24 24"><path d="M12 19V5"/><path d="m6 11 6-6 6 6"/></symbol>
    <symbol id="i-down" viewBox="0 0 24 24"><path d="M12 5v14"/><path d="m6 13 6 6 6-6"/></symbol>
    <symbol id="i-upload" viewBox="0 0 24 24"><path d="M12 15V4"/><path d="m7.5 8.5 4.5-4.5 4.5 4.5"/><path d="M4 14.5V18a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-3.5"/></symbol>
    <symbol id="i-plus" viewBox="0 0 24 24"><path d="M12 5v14M5 12h14"/></symbol>
    <symbol id="i-x" viewBox="0 0 24 24"><path d="m6.5 6.5 11 11M17.5 6.5l-11 11"/></symbol>
    <symbol id="i-copy" viewBox="0 0 24 24"><rect x="8.5" y="8.5" width="12" height="12" rx="2.5"/><path d="M15.5 8.5v-3a2 2 0 0 0-2-2h-8a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h3"/></symbol>
    <symbol id="i-sun" viewBox="0 0 24 24"><circle cx="12" cy="12" r="4"/><path d="M12 2.5v2M12 19.5v2M2.5 12h2M19.5 12h2M5.3 5.3l1.4 1.4M17.3 17.3l1.4 1.4M5.3 18.7l1.4-1.4M17.3 6.7l1.4-1.4"/></symbol>
    <symbol id="i-moon" viewBox="0 0 24 24"><path d="M19.5 14.6A7.8 7.8 0 1 1 9.4 4.5a6.3 6.3 0 0 0 10.1 10.1z"/></symbol>
    <symbol id="i-auto" viewBox="0 0 24 24"><circle cx="12" cy="12" r="8.5"/><path d="M12 3.5v17a8.5 8.5 0 0 0 0-17z" fill="currentColor"/></symbol>
    <symbol id="kin-mark" viewBox="0 0 40 40">
      <circle cx="15" cy="21" r="9.5" fill="#FFF6EF" stroke="#3A1A0E" stroke-width="2.6"/>
      <circle cx="25.5" cy="18" r="9.5" fill="#FFD3BA" stroke="#3A1A0E" stroke-width="2.6"/>
      <circle cx="22.5" cy="17" r="1.6" fill="#3A1A0E"/><circle cx="28.5" cy="17" r="1.6" fill="#3A1A0E"/>
      <path d="M23 21.2q2.5 2.2 5 0" fill="none" stroke="#3A1A0E" stroke-width="2.2" stroke-linecap="round"/>
      <circle cx="11.5" cy="21" r="1.5" fill="#3A1A0E"/>
    </symbol>
    <symbol id="il-t" viewBox="0 0 146 104">
      <ellipse class="il-shadow" cx="66" cy="98" rx="62" ry="5"/>
      <rect class="il-body" x="10" y="26" width="74" height="50" rx="11"/>
      <path class="il-body" d="M3 80a4 4 0 0 1 4-4h80a4 4 0 0 1 4 4v3a6 6 0 0 1-6 6H9a6 6 0 0 1-6-6v-3z"/>
      <circle class="il-cheek" cx="29" cy="55" r="4.5"/><circle class="il-cheek" cx="65" cy="55" r="4.5"/>
      <path class="il-dash" d="M72 22q22-22 44 14"/>
    </symbol>
    <symbol id="il-t-happy" viewBox="0 0 146 104">
      <circle class="il-eye" cx="37" cy="47" r="3.2"/><circle class="il-eye" cx="57" cy="47" r="3.2"/>
      <path class="il-ink" d="M41 56q6 5 12 0"/>
      <path class="il-heart" d="M93 21.5s-7-4.1-7-9.4A4.1 4.1 0 0 1 93 9.4a4.1 4.1 0 0 1 7 2.7c0 5.3-7 9.4-7 9.4z"/>
      <path class="il-spark" d="M106 6v5M103.5 8.5h5"/>
    </symbol>
    <symbol id="il-t-look" viewBox="0 0 146 104">
      <circle class="il-eye" cx="39" cy="46" r="3.2"/><circle class="il-eye" cx="59" cy="46" r="3.2"/>
      <path class="il-ink" d="M43 57h8"/>
      <circle class="il-eye" cx="86" cy="13" r="2.4"/><circle class="il-eye" cx="94" cy="10" r="2.4" opacity=".6"/><circle class="il-eye" cx="102" cy="11" r="2.4" opacity=".3"/>
    </symbol>
    <symbol id="il-t-sleep" viewBox="0 0 146 104">
      <path class="il-ink" d="M33.5 47.5q3.5 2.5 7 0M53.5 47.5q3.5 2.5 7 0"/>
      <path class="il-ink" d="M42 56q5 3.5 10 0"/>
      <text class="il-zz" x="87" y="17">z</text><text class="il-zz" x="97" y="9" style="font-size:10px">z</text>
    </symbol>
    <symbol id="il-p" viewBox="0 0 146 104">
      <rect class="il-apricot" x="102" y="40" width="36" height="58" rx="10"/>
      <path class="il-ink" d="M115 91h10"/>
    </symbol>
    <symbol id="il-p-happy" viewBox="0 0 146 104">
      <circle class="il-eye" cx="114" cy="62" r="2.8"/><circle class="il-eye" cx="126" cy="62" r="2.8"/>
      <path class="il-ink" d="M116 70q4 4 8 0"/>
    </symbol>
    <symbol id="il-p-look" viewBox="0 0 146 104">
      <circle class="il-eye" cx="112" cy="61" r="2.8"/><circle class="il-eye" cx="124" cy="61" r="2.8"/>
      <path class="il-ink" d="M115 71h6"/>
    </symbol>
    <symbol id="il-p-sleep" viewBox="0 0 146 104">
      <path class="il-ink" d="M111 62.5q3 2 6 0M123 62.5q3 2 6 0"/>
      <path class="il-ink" d="M117 70q3 2.5 6 0"/>
    </symbol>
    <symbol id="il-box" viewBox="0 0 170 128">
      <ellipse class="il-shadow" cx="85" cy="120" rx="54" ry="5.5"/>
      <path class="il-honey" d="M48 62 30 40h48l12 18z"/>
      <path class="il-honey" d="M122 62l18-22H92l-12 18z"/>
      <rect class="il-body" x="44" y="56" width="82" height="60" rx="14"/>
      <circle class="il-eye" cx="72" cy="80" r="3.4"/><circle class="il-eye" cx="98" cy="80" r="3.4"/>
      <circle class="il-cheek" cx="63" cy="89" r="5"/><circle class="il-cheek" cx="107" cy="89" r="5"/>
      <path class="il-ink" d="M77 89q8 6.5 16 0"/>
      <g class="bob">
        <path class="il-body" d="M70 8h17l8 8v22a3 3 0 0 1-3 3H70a3 3 0 0 1-3-3V11a3 3 0 0 1 3-3z" transform="rotate(-10 81 24)"/>
        <path class="il-ink" d="M73 22h13M73 28h9" transform="rotate(-10 81 24)"/>
      </g>
      <path class="il-heart" d="M128 30s-6-3.5-6-8a3.5 3.5 0 0 1 6-2.3 3.5 3.5 0 0 1 6 2.3c0 4.5-6 8-6 8z"/>
      <path class="il-spark" d="M38 22v6M35 25h6M146 52v5M143.5 54.5h5"/>
    </symbol>
    <symbol id="pl-pair" viewBox="0 0 146 104">
      <rect class="pl" x="24" y="28" width="56" height="38" rx="6"/>
      <path class="pl" d="M16 76h72"/>
      <rect class="pl" x="98" y="32" width="28" height="46" rx="6"/>
      <path class="pl" d="M108 70h8"/>
      <path class="pl" d="M84 50h10" style="stroke-dasharray:2 4"/>
    </symbol>
    <symbol id="il-vinyl" viewBox="0 0 50 50">
      <circle cx="25" cy="25" r="24" fill="#2B1A14"/>
      <circle cx="25" cy="25" r="20" fill="none" stroke="#3A261E" stroke-width="2"/>
      <circle cx="25" cy="25" r="15.5" fill="none" stroke="#3A261E" stroke-width="2"/>
      <circle cx="25" cy="25" r="10" fill="#F2804A"/>
      <circle cx="25" cy="25" r="2.5" fill="#2B1A14"/>
    </symbol>
</defs></svg>`;
