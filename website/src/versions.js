// Horizon's release tags (docs/versioning.md). From 0.2 the game is vX.Y.Z and the launcher launcher@X.Y.Z;
// before, vYYYY.MM.DD[.N] and launcher-vX.Y.Z. Every 0.2 tag is newer than every tag from before. Shared by
// build.mjs and main.js, and the same order as the launcher's and tools/release_train.py's.

const SEMVER = /^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/;
// Semver's majors stay below this; a date tag's first number is a year.
const FIRST_YEAR = 1000;

const semver = (version) => {
  const m = SEMVER.exec(version);
  return m && Number(m[1]) < FIRST_YEAR ? m : null;
};

export const isGame = (tag) => /^v\d{4}\.\d{1,2}\.\d{1,2}(\.\d+)?$/.test(tag) || (tag.startsWith("v") && !!semver(tag.slice(1)));

// The version in a launcher tag (launcher@0.2.0, launcher-v0.16.0), or null.
export function launcherVersion(tag) {
  if (tag.startsWith("launcher@") && semver(tag.slice(9))) return tag.slice(9);
  const old = /^launcher-v(\d+\.\d+\.\d+)$/.exec(tag);
  return old ? old[1] : null;
}

export const isLauncher = (tag) => launcherVersion(tag) !== null;

// [line, numbers, pre-release]: tags from before 0.2 by their numbers, then 0.2 tags in semver's order.
function key(tag) {
  const version = tag.startsWith("launcher@") ? tag.slice(9) : tag.startsWith("v") ? tag.slice(1) : null;
  const m = version !== null && semver(version);
  if (!m) return [0, (tag.match(/\d+/g) || []).map(Number), []];
  const pre = m[4] ? m[4].split(".").map((id) => (/^\d+$/.test(id) ? [0, Number(id)] : [1, id])) : null;
  return [1, [m[1], m[2], m[3]].map(Number), pre];
}

const cmpList = (a, b, cmp) => {
  for (let i = 0; i < Math.min(a.length, b.length); i++) {
    const d = cmp(a[i], b[i]);
    if (d) return d;
  }
  return a.length - b.length;
};
const cmpNumbers = (a, b) => {
  const strip = (l) => { const c = [...l]; while (c.length && c[c.length - 1] === 0) c.pop(); return c; };
  return cmpList(strip(a), strip(b), (x, y) => x - y);
};
const cmpId = ([ka, va], [kb, vb]) => ka - kb || (ka ? (va < vb ? -1 : va > vb ? 1 : 0) : va - vb);

// Negative when tag a is older than tag b.
export function compareTags(a, b) {
  const [la, na, pa] = key(a);
  const [lb, nb, pb] = key(b);
  if (la !== lb) return la - lb;
  const d = cmpNumbers(na, nb);
  if (d || !la) return d;
  // A release follows its pre-releases.
  if (!pa || !pb) return (pa ? -1 : 0) - (pb ? -1 : 0);
  return cmpList(pa, pb, cmpId);
}

export const newestFirst = (a, b) => compareTags(b, a);
