// WS 连接层：连接、配对流程、断线重连（指数退避 + 页面可见性触发）、心跳 RTT。

export interface WelcomeCfg {
  sens: number;
  natural: boolean;
  scroll_speed: number;
  tap_enabled: boolean;
  g3_up: string;
  g3_down: string;
  g3_left: string;
  g3_right: string;
}

export type ConnState = "connecting" | "pending" | "paired" | "offline" | "busy" | "need_token";

export interface ConnEvents {
  onState: (s: ConnState, detail?: string) => void;
  onRtt: (ms: number) => void;
  onWelcome: (cfg: WelcomeCfg, trusted: boolean) => void;
  onPaired: (name: string) => void;
  onPinNeeded: () => void;
  onToast: (msg: string) => void;
}

export class PalmConn {
  private ws: WebSocket | null = null;
  private retry = 0;
  private retryTimer = 0;
  private pingTimer = 0;
  private lastPingSent = 0;
  private closedByUs = false;
  /** 本次连接的配对尝试进度（0=cred 1=open 2=token 3=放弃），防回环 */
  private pairStage = 0;

  readonly cred: string | null;
  readonly token: string | null;

  constructor(
    private ev: ConnEvents,
    cred: string | null,
    token: string | null,
    /** 稳定设备标识：重复配对时服务端按它去重，一台设备只占一条已配对记录 */
    private deviceId: string | null = null,
  ) {
    this.cred = cred;
    this.token = token;
    document.addEventListener("visibilitychange", () => {
      if (!document.hidden && !this.isOpen()) this.connect();
    });
    window.addEventListener("online", () => this.connect());
  }

  private wsUrl(): string {
    const proto = location.protocol === "https:" ? "wss:" : "ws:";
    return `${proto}//${location.host}/ws`;
  }

  isOpen(): boolean {
    return !!this.ws && this.ws.readyState === WebSocket.OPEN;
  }

  connect() {
    this.closedByUs = false;
    clearTimeout(this.retryTimer);
    if (this.ws && (this.ws.readyState === WebSocket.OPEN || this.ws.readyState === WebSocket.CONNECTING)) {
      return;
    }
    this.ev.onState("connecting");
    const ws = new WebSocket(this.wsUrl());
    ws.binaryType = "arraybuffer";
    this.ws = ws;

    ws.onopen = () => {
      this.retry = 0;
      this.send({
        t: "hello",
        v: 1,
        ua: navigator.userAgent,
        w: innerWidth,
        h: innerHeight,
        dpr: devicePixelRatio,
        touch: navigator.maxTouchPoints > 0,
        device_id: this.deviceId ?? undefined,
      });
    };
    ws.onmessage = (m) => this.onMsg(m.data);
    ws.onclose = () => {
      this.stopPing();
      if (this.closedByUs) return;
      this.ev.onState("offline");
      const delay = Math.min(300 * 2 ** this.retry++, 2000);
      this.retryTimer = setTimeout(() => this.connect(), delay) as unknown as number;
    };
    ws.onerror = () => ws.close();
  }

  private onMsg(data: unknown) {
    if (typeof data !== "string") return;
    let v: any;
    try {
      v = JSON.parse(data);
    } catch {
      return;
    }
    switch (v.t) {
      case "welcome":
        this.ev.onState("pending");
        this.ev.onWelcome(v.cfg, !!v.trusted);
        // 配对逐级回退：长期凭证（免配）→ 免配对码直连 → 一次性 token（旧服务端兜底）
        this.pairStage = 0;
        this.attemptPair();
        break;
      case "pair_ok":
        this.ev.onState("paired");
        if (v.cred) {
          // 服务端签发长期设备凭证（§7：只存手机 + 服务端设备表）；
          // 重复配对会轮换 cred，这里始终以服务端最新为准
          localStorage.setItem("palm.cred", String(v.cred));
        }
        // 一次性 token 已消费：留在本地只会造成下次"二维码已失效"的误报
        localStorage.removeItem("palm.token");
        this.ev.onPaired(String(v.name ?? "device"));
        this.startPing();
        break;
      case "pair_need_pin":
        this.ev.onPinNeeded();
        break;
      case "pair_err":
        if (v.e === "unknown_device") {
          // 凭证被吊销/轮换：清掉后自动走下一级（open/token），无需用户重新扫码
          localStorage.removeItem("palm.cred");
          this.attemptPair();
        } else if (v.e === "open_disabled" || v.e === "bad_mode") {
          // 电脑端关闭了免配对码直连（或旧版服务端）：退回 token 扫码
          this.pairStage = Math.max(this.pairStage, 2);
          this.attemptPair();
        } else if (v.e === "bad_token") {
          localStorage.removeItem("palm.token");
          this.pairStage = 3;
          this.ev.onState("need_token", "配对已失效，请刷新电脑端二维码后重扫");
        } else if (v.e === "bad_pin") {
          this.ev.onToast("PIN 错误，请重试");
        } else {
          this.pairStage = 3;
          this.ev.onState("need_token", `配对失败：${v.e}`);
        }
        break;
      case "pong": {
        const rtt = performance.now() - this.lastPingSent;
        this.ev.onRtt(Math.round(rtt));
        break;
      }
      case "err":
        if (v.e === "busy") {
          this.ev.onState("busy", v.m ?? "有设备正在控制");
        } else if (v.e === "no_accessibility") {
          this.ev.onToast("电脑端缺少辅助功能权限，无法注入输入");
        } else if (v.e === "rate_limited") {
          this.ev.onToast("发送过快被限流，请稍候");
        } else if (v.e === "pair_timeout") {
          this.ev.onState("need_token", "配对超时，请重新扫码");
        }
        break;
    }
  }

  /** 逐级配对：cred → open（免配对码直连）→ token，失败自动降级 */
  private attemptPair() {
    const cred = localStorage.getItem("palm.cred");
    if (this.pairStage === 0 && cred) {
      this.pairStage = 1;
      this.send({ t: "pair", mode: "cred", cred });
      return;
    }
    if (this.pairStage <= 1) {
      this.pairStage = 2;
      this.send({ t: "pair", mode: "open" });
      return;
    }
    if (this.pairStage === 2 && this.token) {
      this.pairStage = 3;
      this.send({ t: "pair", mode: "token", token: this.token });
      return;
    }
    this.pairStage = 3;
    this.ev.onState("need_token", "无法自动配对：请用电脑端二维码扫码");
  }

  /** PIN 二次确认（§9.1：token/open 已验证后） */
  submitPin(pin: string) {
    this.send({ t: "pair", mode: "pin", pin });
  }

  private startPing() {
    this.stopPing();
    this.pingTimer = setInterval(() => {
      if (!this.isOpen()) return;
      this.lastPingSent = performance.now();
      this.send({ t: "ping", ts: Date.now() });
    }, 5000) as unknown as number;
  }

  private stopPing() {
    clearInterval(this.pingTimer);
  }

  send(obj: object) {
    if (this.isOpen()) this.ws!.send(JSON.stringify(obj));
  }

  sendBin(bytes: Uint8Array) {
    if (this.isOpen()) this.ws!.send(bytes);
  }

  close() {
    this.closedByUs = true;
    this.stopPing();
    this.ws?.close();
  }
}
