// Inline SVG icons. Stroke icons for chrome/sidebar, filled ones for entries.
const stroke = {
  back: '<path d="M15 18l-6-6 6-6"/>',
  forward: '<path d="M9 18l6-6-6-6"/>',
  up: '<path d="M12 19V5M5 12l7-7 7 7"/>',
  refresh: '<path d="M21 12a9 9 0 1 1-2.64-6.36L21 8"/><path d="M21 3v5h-5"/>',
  eye: '<path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12z"/><circle cx="12" cy="12" r="3"/>',
  chevron: '<path d="M9 6l6 6-6 6"/>',
  link: '<path d="M7 17L17 7M9 7h8v8"/>',
  home: '<path d="M3 10.5 12 3l9 7.5V21h-6v-6H9v6H3z"/>',
  desktop: '<rect x="2" y="3" width="20" height="14" rx="2"/><path d="M8 21h8M12 17v4"/>',
  documents: '<path d="M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z"/><path d="M14 3v6h6M8 13h8M8 17h6"/>',
  downloads: '<path d="M12 3v12M7 10l5 5 5-5M5 21h14"/>',
  music: '<path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/>',
  pictures: '<rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="9" cy="9" r="2"/><path d="M21 15l-5-5L5 21"/>',
  videos: '<rect x="2" y="6" width="14" height="12" rx="2"/><path d="M16 10l6-4v12l-6-4z"/>',
  trash: '<path d="M3 6h18M8 6V4h8v2M6 6l1 15h10l1-15"/>',
  drive: '<path d="M22 12H2"/><path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/><path d="M6 16h.01M10 16h.01"/>',
  removable: '<path d="M8 2h8v6H8z"/><path d="M6 8h12v12a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2z"/><path d="M11 4.5h.01M13 4.5h.01"/>',
  network: '<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18"/>',
  split: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M12 4v16"/>',
  viewIcons: '<rect x="4" y="4" width="6" height="6" rx="1"/><rect x="14" y="4" width="6" height="6" rx="1"/><rect x="4" y="14" width="6" height="6" rx="1"/><rect x="14" y="14" width="6" height="6" rx="1"/>',
  viewDetails: '<path d="M8 6h13M8 12h13M8 18h13M3.5 6h.01M3.5 12h.01M3.5 18h.01"/>',
  filter: '<path d="M3 5h18l-7 8.5V19l-4 2v-7.5z"/>',
  close: '<path d="M6 6l12 12M18 6 6 18"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  check: '<path d="M5 12.5l4.5 4.5L19 7"/>',
};
stroke.root = stroke.drive;
stroke.folder = '<path d="M3 6a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>';

// Colour stripe per file kind on a common "page" glyph.
const kindColor = {
  image: "#f67400", audio: "#1abc9c", video: "#da4453", archive: "#c9a92f",
  document: "#3daee9", text: "#7f8c8d", code: "#9b59b6", font: "#e67e22",
  app: "#27ae60", file: "#95a5a6", broken: "#da4453",
};

const svg = (cls, body, vb = "0 0 24 24") =>
  `<svg class="icon ${cls}" viewBox="${vb}" aria-hidden="true">${body}</svg>`;

function ui(name) {
  return svg("stroke", stroke[name] || stroke.folder);
}

function entry(icon) {
  if (icon === "folder") {
    return svg("", `
      <path fill="var(--folder-tab)" d="M2 5.5A1.5 1.5 0 0 1 3.5 4h5.4l2 2h9.6A1.5 1.5 0 0 1 22 7.5V9H2z"/>
      <path fill="var(--folder)" d="M2 8.5A1.5 1.5 0 0 1 3.5 7h17A1.5 1.5 0 0 1 22 8.5v10a1.5 1.5 0 0 1-1.5 1.5h-17A1.5 1.5 0 0 1 2 18.5z"/>`);
  }
  if (icon === "app") {
    return svg("", `<rect x="3" y="3" width="18" height="18" rx="5" fill="${kindColor.app}"/>
      <path d="M8 16l4-8 4 8M9.5 13h5" stroke="#fff" stroke-width="1.8" fill="none" stroke-linecap="round"/>`);
  }
  const c = kindColor[icon] || kindColor.file;
  const dash = icon === "broken" ? ' stroke-dasharray="2 1.5"' : "";
  return svg("", `
    <path d="M6 2.5h8l5 5V20a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 5 20V4a1.5 1.5 0 0 1 1-1.5z" fill="var(--page)" stroke="var(--page-edge)" stroke-width="1"${dash}/>
    <path d="M14 2.5V7.5h5" fill="none" stroke="var(--page-edge)" stroke-width="1"/>
    <rect x="5.5" y="15" width="13" height="5" rx="1" fill="${c}"/>`);
}

export const Icons = { ui, entry };
