// Display formatting, roughly matching Dolphin's details view.

const UNITS = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];

function bytes(n) {
  if (n == null) return "";
  if (n < 1024) return `${n} B`;
  let i = 0;
  let v = n;
  while (v >= 1024 && i < UNITS.length - 1) { v /= 1024; i++; }
  return `${v.toFixed(v < 10 ? 1 : 0)} ${UNITS[i]}`;
}

function items(n) {
  if (n == null) return "";
  return n === 1 ? "1 item" : `${n} items`;
}

const timeFmt = new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" });
const dateFmt = new Intl.DateTimeFormat(undefined, { year: "numeric", month: "short", day: "numeric" });

function date(ms) {
  if (ms == null) return "";
  const d = new Date(ms);
  const startOfToday = new Date();
  startOfToday.setHours(0, 0, 0, 0);
  const day = 86400000;
  if (d >= startOfToday) return `Today at ${timeFmt.format(d)}`;
  if (d >= startOfToday - day) return `Yesterday at ${timeFmt.format(d)}`;
  return `${dateFmt.format(d)}, ${timeFmt.format(d)}`;
}

export const Fmt = { bytes, items, date };
