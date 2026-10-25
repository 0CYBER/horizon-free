// /mods/submit: signs in with GitHub through the registry's service, then sends the page and the file. The service
// checks the file and answers with the pull request it opened, or with what to fix.
import { API, entryFrom } from "./mods.js";
import { setUpMenu, setUpTheme } from "./chrome.js";
import { escape } from "./markdown.js";

const auth = document.getElementById("submit-auth");
const form = document.getElementById("submit-form");
const result = document.getElementById("submit-result");
let csrf = "";
// The form's own controls by name (form.id and form.name are the form's attributes).
const control = (name) => form.elements.namedItem(name);

function signedOut(message) {
  auth.innerHTML = `<p class="meta">${message}</p><p class="mod-actions"><a class="button primary" href="${API}/auth/login">Sign in with GitHub</a></p><p class="meta">It only reads your public profile: your GitHub name goes on the pull request and in the project's owners.</p>`;
  form.hidden = true;
}

async function start() {
  let me;
  try {
    const r = await fetch(`${API}/me`, { credentials: "same-origin" });
    if (r.status === 401) return signedOut("Sign in to submit: reviewers and updates go through your GitHub account.");
    if (!r.ok) throw new Error(String(r.status));
    me = await r.json();
  } catch {
    auth.innerHTML = `<p class="meta">The form isn't open right now. You can <a href="https://github.com/ludovic111/horizon-mods">open a pull request by hand</a>.</p>`;
    return;
  }
  csrf = me.csrf;
  auth.innerHTML = `<p class="meta">Signed in as <strong>${escape(me.login)}</strong>. <button class="button ghost small" type="button" id="sign-out">Sign out</button></p>`;
  document.getElementById("sign-out").addEventListener("click", async () => {
    await fetch(`${API}/auth/logout`, { method: "POST", credentials: "same-origin" });
    signedOut("Signed out.");
  });
  form.hidden = false;
}

// A file named <id>-<version>.<extension> fills in the two fields and the kind.
form.querySelector("#f-file").addEventListener("change", (e) => {
  const name = e.target.files[0]?.name || "";
  const m = name.match(/^([a-z0-9_]{1,64})-([A-Za-z0-9][A-Za-z0-9._-]*)\.(hmod|hpack|zip)$/);
  const kind = { hmod: "mod", hpack: "modpack", zip: "resourcepack" }[name.split(".").pop()];
  if (kind) control("kind").value = kind;
  if (m) {
    if (!control("id").value) control("id").value = m[1];
    if (!control("version").value) control("version").value = m[2];
  }
});

form.addEventListener("submit", async (e) => {
  e.preventDefault();
  const send = form.querySelector("#f-send");
  const values = Object.fromEntries(["id", "kind", "name", "summary", "description", "authors", "license", "source", "icon", "key", "version", "game", "changelog"].map((k) => [k, control(k).value]));
  const sig = form.querySelector("#f-signature").files[0];
  values.signature = sig ? await sig.text() : "";
  const body = new FormData();
  body.append("entry", JSON.stringify(entryFrom(values)));
  body.append("file", form.querySelector("#f-file").files[0]);
  send.disabled = true;
  result.innerHTML = `<p class="meta">Checking the file and opening the pull request…</p>`;
  try {
    const r = await fetch(`${API}/submit`, { method: "POST", body, credentials: "same-origin", headers: { "X-CSRF-Token": csrf } });
    const json = await r.json().catch(() => ({}));
    if (r.ok && /^https:\/\/github\.com\//.test(json.pull_request || "")) {
      result.innerHTML = `<p class="lede">Submitted. Follow the review in <a href="${escape(json.pull_request)}">its pull request</a>: the checks' report is there, and reviewers answer there.</p>`;
      form.reset();
    } else {
      result.innerHTML = `<p class="mod-error">${escape(json.error || `The service answered ${r.status}.`)}</p>`;
    }
  } catch {
    result.innerHTML = `<p class="mod-error">The service didn't answer. Try again in a minute.</p>`;
  } finally {
    send.disabled = false;
  }
});

setUpTheme();
setUpMenu();
start();
