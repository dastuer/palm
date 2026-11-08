// Palm 前端入口：token 解析、连接/配对、面板切换、Wake Lock、PWA 注册。

import { GestureEngine } from "./gesture";
import { KeyboardCtl } from "./keyboard";
import { Settings } from "./settings";
import { PalmConn, type WelcomeCfg } from "./ws";

// ---- token（一次性配对凭证，URL #/t=xxx 承载）----
const hashTok = location.hash.match(/t=([0-9a-f]{8,64})/)?.[1];
if (hashTok) {
  localStorage.setItem("palm.token", hashTok);
  history.replaceState(null, "", location.pathname + location.search);
}
const token = localStorage.getItem("palm.token");
const cred = localStorage.getItem("palm.cred");

// ---- 稳定设备标识（同一浏览器重复配对只占用一条已配对设备记录）----
let deviceId = localStorage.getItem("palm.device");
if (!deviceId) {
  // crypto.randomUUID 仅安全上下文可用；LAN 下是 http，需回退
  deviceId = typeof crypto.randomUUID === "function"
    ? crypto.randomUUID()
    : `d-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 12)}`;
  localStorage.setItem("palm.device", deviceId);
}

// ---- DOM ----
const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const pad = $("touchpad");
const dot = $("dot");
const rttEl = $("rtt");
const toastEl = $("toast");

let toastTimer = 0;
function toast(msg: string) {
  toastEl.textContent = msg;
  toastEl.classList.add("show");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => toastEl.classList.remove("show"), 2600) as unknown as number;
}

// ---- 状态与控件 ----
const settings = new Settings();

const engine = new GestureEngine(
  pad,
  (f) => conn.sendBin(f),
  {
    sens: settings.cfg.sens,
    accelK: settings.cfg.precise ? 0 : 1.8,
    scrollSpeed: settings.cfg.scrollSpeed,
    tapEnabled: settings.cfg.tapEnabled,
    longPressMs: settings.cfg.longPressMs,
  },
);

const conn = new PalmConn(
  {
    onState: (s, detail) => {
      dot.className = { paired: "ok", connecting: "off", pending: "pairing", offline: "off", busy: "off", need_token: "off" }[s] ?? "off";
      rttEl.textContent = s === "paired" ? rttEl.textContent : "—";
      if (s === "need_token" || s === "busy") {
        $("paircard").classList.remove("hidden");
        $("pair-status").textContent = detail ?? "";
        if (s === "need_token") {
          localStorage.removeItem("palm.token");
        }
      } else {
        $("paircard").classList.add("hidden");
      }
    },
    onRtt: (ms) => {
      rttEl.textContent = `${ms}ms`;
    },
    onWelcome: (cfg: WelcomeCfg, trusted: boolean) => {
      settings.applyServer(cfg);
      if (!trusted) toast("电脑端缺少辅助功能权限，输入注入暂不可用");
    },
    onPaired: (name) => {
      $("paircard").classList.add("hidden");
      $("pinentry").classList.add("hidden");
      toast(`已连接：${name}`);
    },
    onPinNeeded: () => {
      $("pinentry").classList.remove("hidden");
      $("pin-input").focus();
    },
    onToast: toast,
  },
  cred,
  token,
  deviceId,
);

settings.onChange = (s) => {
  engine.setOpts({
    sens: s.sens,
    accelK: s.precise ? 0 : 1.8,
    scrollSpeed: s.scrollSpeed,
    tapEnabled: s.tapEnabled,
    longPressMs: s.longPressMs,
  });
  conn.send(settings.serverPatch());
};

// ---- 键盘 ----
const kb = new KeyboardCtl((o) => conn.send(o), {
  keybar: $("keybar"),
  keysPanel: $("keys"),
  ime: $("ime") as HTMLInputElement,
  onToast: toast,
});
$("ime-toggle").addEventListener("click", () => kb.focusIme());
$("pin-go").addEventListener("click", () => {
  const pin = ($("pin-input") as HTMLInputElement).value.trim();
  if (/^\d{4}$/.test(pin)) {
    conn.submitPin(pin);
    $("pinentry").classList.add("hidden");
  } else {
    toast("PIN 为 4 位数字");
  }
});
$("pin-input").addEventListener("keydown", (e) => {
  if (e.key === "Enter") $("pin-go").click();
});

// ---- 面板切换 ----
const panels: Array<[string, HTMLElement]> = [
  ["tab-touch", pad],
  ["tab-keys", $("keys")],
  ["tab-cfg", $("cfg")],
];
for (const [tabId, panel] of panels) {
  $(tabId).addEventListener("click", () => {
    for (const [t2, p2] of panels) {
      $(t2).classList.toggle("on", t2 === tabId);
      p2.classList.toggle("hidden", p2 !== panel);
    }
    if (tabId !== "tab-keys") kb.blurIme();
  });
}

// ---- 配对卡片 ----
$("pair-go").addEventListener("click", () => {
  const url = ($("pair-url") as HTMLInputElement).value.trim();
  const m = url.match(/t=([0-9a-f]{8,64})/);
  if (m) {
    localStorage.setItem("palm.token", m[1]);
    location.reload();
  } else {
    toast("链接里没有 #/t=… token");
  }
});
$("pair-close").addEventListener("click", () => $("paircard").classList.add("hidden"));

// ---- Wake Lock（防息屏，iOS 16.4+）----
async function requestWakeLock() {
  try {
    await (navigator as any).wakeLock?.request("screen");
  } catch {
    /* 不支持或被拒绝 */
  }
}
document.addEventListener("visibilitychange", () => {
  if (!document.hidden) requestWakeLock();
});
requestWakeLock();

// ---- 微信内置浏览器提示（§11）----
if (/MicroMessenger/i.test(navigator.userAgent)) {
  toast("微信内可能无法连接，请用 Safari/Chrome 打开本页");
}

// ---- 启动 ----
conn.connect();

// PWA Service Worker（仅 https 或 localhost 有意义）
if ("serviceWorker" in navigator && (location.protocol === "https:" || ["localhost", "127.0.0.1"].includes(location.hostname))) {
  navigator.serviceWorker.register("/sw.js").catch(() => {});
}
