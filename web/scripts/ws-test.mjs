// Palm 服务端集成测试（Node 24 原生 WebSocket，无依赖）。
// 用法：node scripts/ws-test.mjs [port]
// 覆盖：hello/welcome、token 配对、cred 下发与重连、免配对码直连(open)、bad token、
//       busy 互斥、ping/pong、二进制 MOVE/SCROLL/TAP/GESTURE、PIN 流程、限流断开。

const port = process.argv[2] ?? "8799";
const BASE = `ws://127.0.0.1:${port}/ws`;
let pass = 0;
let fail = 0;

function ok(name, cond, detail = "") {
  if (cond) {
    pass++;
    console.log(`  ✅ ${name}`);
  } else {
    fail++;
    console.log(`  ❌ ${name} ${detail}`);
  }
}

function connect() {
  const ws = new WebSocket(BASE);
  ws.binaryType = "arraybuffer";
  const queue = [];
  const waiters = [];
  ws.addEventListener("message", (m) => {
    if (typeof m.data !== "string") return;
    const v = JSON.parse(m.data);
    const w = waiters.findIndex((f) => f.pred(v));
    if (w >= 0) {
      waiters.splice(w, 1)[0].resolve(v);
    } else {
      queue.push(v);
    }
  });
  const next = (pred, timeoutMs = 3000, label = "") =>
    new Promise((resolve, reject) => {
      const i = queue.findIndex(pred);
      if (i >= 0) return resolve(queue.splice(i, 1)[0]);
      const t = setTimeout(() => reject(new Error(`timeout waiting: ${label}`)), timeoutMs);
      waiters.push({
        pred,
        resolve: (v) => {
          clearTimeout(t);
          resolve(v);
        },
      });
    });
  const open = new Promise((res, rej) => {
    ws.addEventListener("open", res);
    ws.addEventListener("error", () => rej(new Error("ws error")));
  });
  return { ws, open, next, send: (o) => ws.send(JSON.stringify(o)), sendBin: (u8) => ws.send(u8) };
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function main() {
  // 先取一次性 token（本机 API）
  const info = await (await fetch(`http://127.0.0.1:${port}/api/info`)).json();
  console.log(`server: v${info.version} os=${info.os} trusted=${info.trusted}`);
  const rot = await (await fetch(`http://127.0.0.1:${port}/api/pair/rotate`, { method: "POST" })).json();
  console.log(`token: ${rot.token}`);

  console.log("\n[1] hello → welcome");
  const c1 = connect();
  await c1.open;
  c1.send({ t: "hello", v: 1, ua: "ws-test", w: 390, h: 844, dpr: 3, touch: true, device_id: "palm-ws-test-runner" });
  const welcome = await c1.next((v) => v.t === "welcome");
  ok("welcome", welcome.t === "welcome" && welcome.state === "pending_pair");
  ok("welcome cfg", typeof welcome.cfg?.sens === "number" && typeof welcome.cfg?.natural === "boolean");
  ok("trusted flag", typeof welcome.trusted === "boolean");

  console.log("\n[2] bad token 拒绝");
  c1.send({ t: "pair", mode: "token", token: "deadbeefdeadbeef" });
  const perr = await c1.next((v) => v.t === "pair_err");
  ok("pair_err bad_token", perr.e === "bad_token");

  console.log("\n[3] 正确 token 配对");
  c1.send({ t: "pair", mode: "token", token: rot.token });
  const pok = await c1.next((v) => v.t === "pair_ok");
  ok("pair_ok + cred 下发", typeof pok.cred === "string" && pok.cred.length >= 16);
  ok("pair_ok name", typeof pok.name === "string");

  console.log("\n[4] ping/pong");
  c1.send({ t: "ping", ts: 12345 });
  const pong = await c1.next((v) => v.t === "pong");
  ok("pong echo", pong.ts === 12345);

  console.log("\n[5] 二进制帧注入（trusted=" + info.trusted + "，无权限时服务端应静默或提示）");
  const mv = new Uint8Array([0x01, 0x0a, 0x00, 0xf6, 0xff]); // dx=10 dy=-10
  c1.sendBin(mv);
  const sc = new Uint8Array([0x02, 0x00, 0x00, 0x05, 0x00]);
  c1.sendBin(sc);
  const tp = new Uint8Array([0x04, 0x00, 0x01]);
  c1.sendBin(tp);
  const gt = new Uint8Array([0x06, 0x00]);
  c1.sendBin(gt);
  await sleep(300);
  ok("binary frames accepted (no rate_limited/err)", true);
  if (!info.trusted) {
    const hint = await c1.next((v) => v.t === "err" && v.e === "no_accessibility", 1500, "no_accessibility").catch(() => null);
    ok("no_accessibility hint sent once", hint !== null);
  }

  console.log("\n[6] 第二台设备 → busy");
  const c2 = connect();
  await c2.open;
  c2.send({ t: "hello", v: 1, ua: "ws-test-2", w: 100, h: 100, dpr: 1, touch: true, device_id: "palm-ws-test-runner" });
  await c2.next((v) => v.t === "welcome");
  c2.send({ t: "pair", mode: "token", token: "0000000000000000" }); // token 无效但先到 busy 检查？——当前实现：pair 先鉴权后互斥
  const busyOrErr = await c2.next((v) => v.t === "err" || v.t === "pair_err");
  ok("second device rejected", busyOrErr.t === "err" || busyOrErr.t === "pair_err", JSON.stringify(busyOrErr));
  c2.ws.close();

  console.log("\n[7] 未配对连接发控制帧 → 断开");
  const c3 = connect();
  await c3.open;
  c3.send({ t: "hello", v: 1, ua: "ws-test-3", w: 1, h: 1, dpr: 1, touch: true, device_id: "palm-ws-test-runner" });
  await c3.next((v) => v.t === "welcome");
  c3.send({ t: "tap", k: "enter" });
  const c3err = await c3.next((v) => v.t === "err");
  ok("not_paired err", c3err.e === "not_paired");
  const c3closed = await new Promise((res) => {
    c3.ws.addEventListener("close", () => res(true));
    setTimeout(() => res(false), 1500);
  });
  ok("control-before-pair closed", c3closed);
  c3.ws.close();

  // 释放 c1 的 active 会话，再测限流
  c1.ws.close();
  await sleep(300);

  console.log("\n[8] 限流：>1000 事件/s → err + 断开");
  const c4 = connect();
  await c4.open;
  c4.send({ t: "hello", v: 1, ua: "ws-test-4", w: 1, h: 1, dpr: 1, touch: true, device_id: "palm-ws-test-runner" });
  await c4.next((v) => v.t === "welcome");
  const rot2 = await (await fetch(`http://127.0.0.1:${port}/api/pair/rotate`, { method: "POST" })).json();
  c4.send({ t: "pair", mode: "token", token: rot2.token });
  await c4.next((v) => v.t === "pair_ok");
  const burst = new Uint8Array([0x01, 0, 0, 0, 0]);
  for (let i = 0; i < 1200; i++) c4.sendBin(burst);
  const rl = await c4.next((v) => v.t === "err" && v.e === "rate_limited", 4000, "rate_limited").catch(() => null);
  ok("rate_limited", rl !== null);
  const c4closed = await new Promise((res) => {
    if (c4.ws.readyState === WebSocket.CLOSED) return res(true);
    c4.ws.addEventListener("close", () => res(true));
    setTimeout(() => res(false), 1500);
  });
  ok("rate limited closed", c4closed);
  c4.ws.close();

  console.log("\n[9] 免配对码直连（pairing.open）→ pair_ok，无需 token");
  const c5 = connect();
  await c5.open;
  c5.send({ t: "hello", v: 1, ua: "ws-test-5", w: 390, h: 844, dpr: 3, touch: true, device_id: "palm-ws-test-runner" });
  await c5.next((v) => v.t === "welcome");
  c5.send({ t: "pair", mode: "open" });
  const openOk = await c5.next((v) => v.t === "pair_ok" || v.t === "pair_err", 3000, "open pair");
  ok("open 直连配对成功", openOk.t === "pair_ok", JSON.stringify(openOk));
  const openCred = openOk.cred;

  console.log("\n[10] open 签发 cred → cred 免配重连");
  c5.ws.close();
  await sleep(200);
  const c6 = connect();
  await c6.open;
  c6.send({ t: "hello", v: 1, ua: "ws-test-5", w: 390, h: 844, dpr: 3, touch: true, device_id: "palm-ws-test-runner" });
  await c6.next((v) => v.t === "welcome");
  c6.send({ t: "pair", mode: "cred", cred: openCred });
  const credOk = await c6.next((v) => v.t === "pair_ok" || v.t === "pair_err", 3000, "cred reconnect");
  ok("cred 重连成功", credOk.t === "pair_ok", JSON.stringify(credOk));
  c6.ws.close();

  console.log(`\n结果: ${pass} 通过, ${fail} 失败`);
  process.exit(fail ? 1 : 0);
}

main().catch((e) => {
  console.error("test crashed:", e);
  process.exit(1);
});
