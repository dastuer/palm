// 手势状态机（§5.2）：单指点按/移动/长按拖拽、双指滚动+惯性/右键/捏合、三指手势。
// 语义约定：MOVE/SCROLL 都发"位移增量"（CSS px），dy<0 = 手指上移；
// 灵敏度/加速在本层应用于 MOVE，滚动速度应用于 SCROLL；natural 方向由服务端换算。

import {
  moveFrame,
  scrollFrame,
  buttonFrame,
  tapFrame,
  pinchFrame,
  gestureFrame,
  GESTURE_ID,
  BTN,
} from "./proto";

export interface GestureOpts {
  sens: number;
  accelK: number; // 0 = 精确模式
  scrollSpeed: number;
  tapEnabled: boolean;
  longPressMs: number;
}

type Pt = { x: number; y: number };
type State = "idle" | "pending" | "move" | "drag" | "scroll" | "three" | "freeze";

const TAP_MS = 200; // tap 时长阈值
const DEAD_ZONE = 8; // tap 判定死区 px
const PINCH_ENTER = 10; // 距离变化超过该值进入捏合
const THREE_THRESH = 30; // 三指位移阈值
const DBL_TAP_MS = 300;
const INERTIA_LAMBDA = 0.92; // 惯性衰减/帧（§5.2）
const INERTIA_STOP_PX = 2; // 帧位移 <2px 停
const V_MAX = 2.0; // 加速曲线参考速度 px/ms

export class GestureEngine {
  state: State = "idle";
  opts: GestureOpts;

  private el: HTMLElement;
  private send: (frame: Uint8Array) => void;

  private pts = new Map<number, Pt>();
  private mainId = -1;
  private mainAnchor: Pt = { x: 0, y: 0 };
  private downT = 0;
  private acc = { dx: 0, dy: 0 };
  private sacc = { dx: 0, dy: 0 };
  private pacc = 0;
  private longTimer = 0;

  private scrollIds: number[] = [];
  private twoCenter: Pt = { x: 0, y: 0 };
  private twoDist = 0;
  private twoStartT = 0;
  private twoMoved = 0;
  private pinchMode = false;
  private samples: { t: number; x: number; y: number }[] = [];

  private threeAnchor: Pt = { x: 0, y: 0 };
  private threeFired = false;

  private lastTapEnd = 0;
  private rafId = 0;
  private lastRafT = 0;
  private inertiaV = { x: 0, y: 0 };
  private inertiaLastT = 0;
  private inertiaRaf = 0;

  constructor(el: HTMLElement, send: (f: Uint8Array) => void, opts: GestureOpts) {
    this.el = el;
    this.send = send;
    this.opts = opts;

    el.addEventListener("pointerdown", this.onDown);
    el.addEventListener("pointermove", this.onMove);
    el.addEventListener("pointerup", this.onUp);
    el.addEventListener("pointercancel", (e) => this.onUp(e, true));
    // 阻止 iOS 双击缩放/橡皮筋（touch-action:none 之外的兜底）
    for (const t of ["touchstart", "touchmove", "touchend"] as const) {
      el.addEventListener(t, (e) => e.preventDefault(), { passive: false });
    }
    window.addEventListener("pagehide", () => this.releaseAll());
    document.addEventListener("visibilitychange", () => {
      if (document.hidden) this.releaseAll();
    });
  }

  setOpts(o: Partial<GestureOpts>) {
    this.opts = { ...this.opts, ...o };
  }

  destroy() {
    this.el.removeEventListener("pointerdown", this.onDown);
    this.el.removeEventListener("pointermove", this.onMove);
    this.el.removeEventListener("pointerup", this.onUp);
    this.releaseAll();
  }

  private releaseAll() {
    clearTimeout(this.longTimer);
    cancelAnimationFrame(this.rafId);
    cancelAnimationFrame(this.inertiaRaf);
    this.rafId = 0;
    this.inertiaRaf = 0;
    if (this.state === "drag") this.send(buttonFrame(BTN.LEFT, false));
    this.pts.clear();
    this.state = "idle";
  }

  private center(list: Pt[]): Pt {
    let x = 0;
    let y = 0;
    for (const p of list) {
      x += p.x;
      y += p.y;
    }
    return { x: x / list.length, y: y / list.length };
  }

  private onDown = (e: PointerEvent) => {
    try {
      this.el.setPointerCapture?.(e.pointerId);
    } catch {
      /* pointerId 已失效（如指针已释放），忽略 */
    }
    this.pts.set(e.pointerId, { x: e.clientX, y: e.clientY });
    const n = this.pts.size;
    if (n > 3) return; // 4+ 忽略（防误触）

    switch (this.state) {
      case "idle": {
        this.state = "pending";
        this.mainId = e.pointerId;
        this.mainAnchor = { x: e.clientX, y: e.clientY };
        this.downT = performance.now();
        this.acc = { dx: 0, dy: 0 };
        clearTimeout(this.longTimer);
        this.longTimer = setTimeout(() => this.enterDrag(), this.opts.longPressMs) as unknown as number;
        break;
      }
      case "pending":
      case "move": {
        clearTimeout(this.longTimer);
        if (n >= 2) this.enterScroll();
        break;
      }
      case "scroll": {
        if (n >= 3) this.enterThree();
        break;
      }
      default:
        break;
    }
    this.ensureRaf();
  };

  private enterDrag() {
    if (this.state !== "pending") return;
    this.state = "drag";
    this.send(buttonFrame(BTN.LEFT, true));
    navigator.vibrate?.(10); // Android 触觉反馈（§5.2）
    this.acc = { dx: 0, dy: 0 };
  }

  private enterScroll() {
    this.state = "scroll";
    this.acc = { dx: 0, dy: 0 };
    this.sacc = { dx: 0, dy: 0 };
    this.pacc = 0;
    this.pinchMode = false;
    const ids = [...this.pts.keys()].slice(0, 2);
    this.scrollIds = ids;
    const [a, b] = ids.map((id) => this.pts.get(id)!);
    this.twoCenter = { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
    this.twoDist = Math.hypot(a.x - b.x, a.y - b.y);
    this.twoStartT = performance.now();
    this.twoMoved = 0;
    this.samples = [{ t: this.twoStartT, x: this.twoCenter.x, y: this.twoCenter.y }];
  }

  private enterThree() {
    this.state = "three";
    this.threeAnchor = this.center([...this.pts.values()]);
    this.threeFired = false;
  }

  private onMove = (e: PointerEvent) => {
    if (!this.pts.has(e.pointerId)) return;
    const evs = e.getCoalescedEvents?.() ?? [];
    if (evs.length === 0) evs.push(e);
    for (const ce of evs) this.applyMove(ce);
    this.ensureRaf();
  };

  private applyMove(e: PointerEvent) {
    const p = this.pts.get(e.pointerId);
    if (!p) return;
    const dx = e.clientX - p.x;
    const dy = e.clientY - p.y;
    p.x = e.clientX;
    p.y = e.clientY;

    switch (this.state) {
      case "pending": {
        this.acc.dx += dx;
        this.acc.dy += dy;
        const moved = Math.hypot(
          p.x - this.mainAnchor.x,
          p.y - this.mainAnchor.y,
        );
        if (moved > DEAD_ZONE) {
          clearTimeout(this.longTimer);
          this.state = "move";
        }
        break;
      }
      case "move":
      case "drag": {
        this.acc.dx += dx;
        this.acc.dy += dy;
        break;
      }
      case "scroll": {
        if (!this.scrollIds.includes(e.pointerId)) return;
        const [a, b] = this.scrollIds.map((id) => this.pts.get(id));
        if (!a || !b) return;
        const c = { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
        const d = Math.hypot(a.x - b.x, a.y - b.y);
        this.twoMoved += Math.hypot(c.x - this.twoCenter.x, c.y - this.twoCenter.y);
        if (!this.pinchMode && Math.abs(d - this.twoDist) > PINCH_ENTER) {
          this.pinchMode = true;
        }
        if (this.pinchMode) {
          this.pacc += d - this.twoDist;
        } else {
          this.sacc.dx += c.x - this.twoCenter.x;
          this.sacc.dy += c.y - this.twoCenter.y;
        }
        this.twoCenter = c;
        this.twoDist = d;
        const now = performance.now();
        this.samples.push({ t: now, x: c.x, y: c.y });
        while (this.samples.length > 2 && now - this.samples[0].t > 100) this.samples.shift();
        break;
      }
      case "three": {
        if (this.threeFired) return;
        const c = this.center([...this.pts.values()]);
        const dx3 = c.x - this.threeAnchor.x;
        const dy3 = c.y - this.threeAnchor.y;
        if (Math.hypot(dx3, dy3) > THREE_THRESH) {
          this.threeFired = true;
          if (Math.abs(dx3) > Math.abs(dy3)) {
            this.send(gestureFrame(dx3 > 0 ? GESTURE_ID.RIGHT : GESTURE_ID.LEFT));
          } else {
            this.send(gestureFrame(dy3 > 0 ? GESTURE_ID.DOWN : GESTURE_ID.UP));
          }
        }
        break;
      }
      default:
        break;
    }
  }

  private onUp = (e: PointerEvent, cancel = false) => {
    if (!this.pts.has(e.pointerId)) return;
    this.pts.delete(e.pointerId);
    const now = performance.now();

    switch (this.state) {
      case "pending": {
        clearTimeout(this.longTimer);
        const main = this.pts.get(this.mainId);
        const dt = now - this.downT;
        const moved = main
          ? Math.hypot(main.x - this.mainAnchor.x, main.y - this.mainAnchor.y)
          : Infinity;
        if (!cancel && this.opts.tapEnabled && dt < TAP_MS && moved <= DEAD_ZONE) {
          const count = now - this.lastTapEnd < DBL_TAP_MS ? 2 : 1;
          this.send(tapFrame(BTN.LEFT, count));
          this.lastTapEnd = now;
        }
        if (this.pts.size === 1) {
          // 剩余单指重新进入 pending
          const [id] = this.pts.keys();
          const pt = this.pts.get(id)!;
          this.state = "pending";
          this.mainId = id;
          this.mainAnchor = { ...pt };
          this.downT = now;
          this.longTimer = setTimeout(() => this.enterDrag(), this.opts.longPressMs) as unknown as number;
        } else {
          this.state = "idle";
        }
        break;
      }
      case "move": {
        if (this.pts.size >= 2) this.enterScroll();
        else if (this.pts.size === 0) this.state = "idle";
        break;
      }
      case "drag": {
        if (this.pts.size === 0) {
          this.send(buttonFrame(BTN.LEFT, false));
          this.state = "idle";
        }
        break;
      }
      case "scroll": {
        if (this.pts.size === 0) {
          // 双指点按 → 右键（§5.2）
          if (!cancel && now - this.twoStartT < TAP_MS && this.twoMoved < DEAD_ZONE) {
            this.send(tapFrame(BTN.RIGHT, 1));
          } else {
            this.startInertia();
          }
          this.state = "idle";
        } else {
          // 抬起一指：滚动结束、惯性继续，剩余手指冻结
          if (!cancel) this.startInertia();
          this.state = "freeze";
        }
        break;
      }
      case "three":
      case "freeze": {
        if (this.pts.size === 0) this.state = "idle";
        break;
      }
      default:
        break;
    }
    this.ensureRaf();
  };

  // ---- rAF 合帧（§9.2：每帧每类最多 1 帧，不丢位移）----

  private ensureRaf() {
    if (this.rafId) return;
    this.lastRafT = performance.now();
    this.rafId = requestAnimationFrame(this.flush);
  }

  private flush = (t: number) => {
    this.rafId = 0;
    const dt = Math.min(50, Math.max(1, t - this.lastRafT));
    this.lastRafT = t;

    if (this.state === "pending" || this.state === "move" || this.state === "drag") {
      if (this.acc.dx !== 0 || this.acc.dy !== 0) {
        const v = Math.hypot(this.acc.dx, this.acc.dy) / dt;
        const k = this.opts.accelK;
        const factor = 1 + k * Math.min(v / V_MAX, 1);
        this.send(
          moveFrame(
            this.acc.dx * this.opts.sens * factor,
            this.acc.dy * this.opts.sens * factor,
          ),
        );
        this.acc = { dx: 0, dy: 0 };
      }
    }
    if (this.state === "scroll") {
      if (this.pinchMode) {
        if (this.pacc !== 0) {
          this.send(pinchFrame(this.pacc, 0, 0));
          this.pacc = 0;
        }
      } else if (this.sacc.dx !== 0 || this.sacc.dy !== 0) {
        this.send(
          scrollFrame(this.sacc.dx * this.opts.scrollSpeed, this.sacc.dy * this.opts.scrollSpeed),
        );
        this.sacc = { dx: 0, dy: 0 };
      }
    }

    if (["pending", "move", "drag", "scroll", "three"].includes(this.state)) {
      this.rafId = requestAnimationFrame(this.flush);
    }
  };

  // ---- 惯性（抬手按速度指数衰减，§5.2）----

  private startInertia() {
    const s = this.samples;
    if (s.length < 2) return;
    const last = s[s.length - 1];
    const first = s[0];
    const dt = last.t - first.t;
    if (dt < 5) return;
    this.inertiaV = {
      x: ((last.x - first.x) / dt) * this.opts.scrollSpeed,
      y: ((last.y - first.y) / dt) * this.opts.scrollSpeed,
    };
    if (Math.hypot(this.inertiaV.x, this.inertiaV.y) < 0.05) return;
    this.inertiaLastT = performance.now();
    cancelAnimationFrame(this.inertiaRaf);
    this.inertiaRaf = requestAnimationFrame(this.inertiaStep);
  }

  private inertiaStep = (t: number) => {
    this.inertiaRaf = 0;
    const dt = Math.min(50, Math.max(1, t - this.inertiaLastT));
    this.inertiaLastT = t;
    const dx = this.inertiaV.x * dt;
    const dy = this.inertiaV.y * dt;
    this.send(scrollFrame(dx, dy));
    const decay = Math.pow(INERTIA_LAMBDA, dt / 16.7);
    this.inertiaV.x *= decay;
    this.inertiaV.y *= decay;
    if (Math.hypot(dx, dy) > INERTIA_STOP_PX && Math.hypot(this.inertiaV.x, this.inertiaV.y) > 0.02) {
      this.inertiaRaf = requestAnimationFrame(this.inertiaStep);
    }
  };
}
