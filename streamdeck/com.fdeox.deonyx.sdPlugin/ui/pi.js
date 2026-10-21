// Settings panel of a key: keeps the fields and the key's settings in step,
// and fills the suggestions with the apps and profiles Deonyx knows.

let socket = null;
let context = "";
let settings = {};

function save() {
  if (socket?.readyState === WebSocket.OPEN) socket.send(JSON.stringify({ event: "setSettings", context, payload: settings }));
}

function suggest(id, values) {
  const list = document.getElementById(id);
  if (list) list.innerHTML = (values || []).map((v) => `<option value="${String(v).replace(/"/g, "&quot;")}">`).join("");
}

// Called by Stream Deck when the panel opens.
// eslint-disable-next-line no-unused-vars
function connectElgatoStreamDeckSocket(port, uuid, registerEvent, info, actionInfo) {
  context = uuid;
  settings = JSON.parse(actionInfo).payload.settings || {};
  document.querySelectorAll("[data-setting]").forEach((el) => {
    const name = el.dataset.setting;
    if (settings[name] !== undefined) el.value = settings[name];
    el.addEventListener("input", () => {
      settings[name] = el.type === "number" ? Number(el.value) : el.value.trim();
      save();
    });
  });
  socket = new WebSocket(`ws://127.0.0.1:${port}`);
  socket.onopen = () => socket.send(JSON.stringify({ event: registerEvent, uuid }));
  socket.onmessage = (e) => {
    const msg = JSON.parse(e.data);
    if (msg.event === "sendToPropertyInspector") {
      suggest("apps", msg.payload?.apps);
      suggest("profiles", msg.payload?.profiles);
    }
  };
}
