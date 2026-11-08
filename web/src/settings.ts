// 设置面板（§5.1 设置项 + §4.2 cfg 同步）。sens/natural/scroll_speed/三指映射
// 持久化到服务端；长按时长/精确模式/主题存 localStorage（纯前端行为）。

export interface PalmSettings {
  sens: number;
  natural: boolean;
  scrollSpeed: number;
  tapEnabled: boolean;
  longPressMs: number;
  precise: boolean; // 关闭加速曲线
  g3up: string;
  g3lr: string; // 三指左右合并设置
  theme: "auto" | "dark" | "light";
}

const DEFAULTS: PalmSettings = {
  sens: 1.0,
  natural: true,
  scrollSpeed: 1.0,
  tapEnabled: true,
  longPressMs: 450,
  precise: false,
  g3up: "mission_control",
  g3lr: "space_left_right",
  theme: "auto",
};

const LS_KEY = "palm.settings";

export class Settings {
  cfg: PalmSettings;
  onChange: (s: PalmSettings) => void = () => {};

  constructor() {
    this.cfg = { ...DEFAULTS, ...this.loadLocal() };
    this.bindDom();
    this.applyTheme();
    this.render();
  }

  /** 服务端 welcome.cfg 覆盖（服务端持久化过的值优先） */
  applyServer(cfg: any) {
    if (!cfg) return;
    if (typeof cfg.sens === "number") this.cfg.sens = cfg.sens;
    if (typeof cfg.natural === "boolean") this.cfg.natural = cfg.natural;
    if (typeof cfg.scroll_speed === "number") this.cfg.scrollSpeed = cfg.scroll_speed;
    if (typeof cfg.tap_enabled === "boolean") this.cfg.tapEnabled = cfg.tap_enabled;
    if (typeof cfg.g3_up === "string" && cfg.g3_up !== "none") this.cfg.g3up = cfg.g3_up;
    if (typeof cfg.g3_left === "string" && typeof cfg.g3_right === "string") {
      if (cfg.g3_left === "space_left" && cfg.g3_right === "space_right") this.cfg.g3lr = "space_left_right";
      else if (cfg.g3_left === "mission_control") this.cfg.g3lr = "mission_control";
      else if (cfg.g3_left === "none") this.cfg.g3lr = "none";
    }
    this.saveLocal();
    this.render();
  }

  /** 发送到服务端的 cfg 消息体（§4.2） */
  serverPatch(): object {
    const [l, r] =
      this.cfg.g3lr === "space_left_right"
        ? ["space_left", "space_right"]
        : this.cfg.g3lr === "mission_control"
          ? ["mission_control", "mission_control"]
          : ["none", "none"];
    // 三指下滑取"另一半"：上滑调度中心 → 下滑 APP 窗口，反之亦然
    const downAction =
      this.cfg.g3up === "mission_control"
        ? "app_expose"
        : this.cfg.g3up === "app_expose"
          ? "mission_control"
          : "none";
    return {
      t: "cfg",
      sens: this.cfg.sens,
      natural: this.cfg.natural,
      scroll_speed: this.cfg.scrollSpeed,
      g3_up: this.cfg.g3up,
      g3_down: downAction,
      g3_left: l,
      g3_right: r,
    };
  }

  private loadLocal(): Partial<PalmSettings> {
    try {
      return JSON.parse(localStorage.getItem(LS_KEY) ?? "{}");
    } catch {
      return {};
    }
  }

  private saveLocal() {
    localStorage.setItem(LS_KEY, JSON.stringify(this.cfg));
  }

  private bindDom() {
    const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

    const sens = $("i-sens") as HTMLInputElement;
    sens.addEventListener("input", () => {
      this.cfg.sens = Number(sens.value);
      this.changed();
    });
    const scroll = $("i-scroll") as HTMLInputElement;
    scroll.addEventListener("input", () => {
      this.cfg.scrollSpeed = Number(scroll.value);
      this.changed();
    });
    const natural = $("i-natural") as HTMLInputElement;
    natural.addEventListener("change", () => {
      this.cfg.natural = natural.checked;
      this.changed();
    });
    const tap = $("i-tap") as HTMLInputElement;
    tap.addEventListener("change", () => {
      this.cfg.tapEnabled = tap.checked;
      this.saveLocal();
    });
    const lp = $("i-lp") as HTMLInputElement;
    lp.addEventListener("input", () => {
      this.cfg.longPressMs = Number(lp.value);
      this.saveLocal();
      this.render();
    });
    const precise = $("i-precise") as HTMLInputElement;
    precise.addEventListener("change", () => {
      this.cfg.precise = precise.checked;
      this.saveLocal();
    });
    const g3u = $("i-g3u") as HTMLSelectElement;
    g3u.addEventListener("change", () => {
      this.cfg.g3up = g3u.value;
      this.changed();
    });
    const g3lr = $("i-g3lr") as HTMLSelectElement;
    g3lr.addEventListener("change", () => {
      this.cfg.g3lr = g3lr.value;
      this.changed();
    });
    const theme = $("i-theme") as HTMLSelectElement;
    theme.addEventListener("change", () => {
      this.cfg.theme = theme.value as PalmSettings["theme"];
      this.applyTheme();
      this.saveLocal();
    });
    const unpair = $("unpair");
    unpair?.addEventListener("click", () => {
      localStorage.removeItem("palm.cred");
      localStorage.removeItem("palm.token");
      this.onChange({ ...this.cfg }); // 触发 UI 提示
      location.hash = "";
      location.reload();
    });
  }

  private changed() {
    this.saveLocal();
    this.render();
    this.onChange({ ...this.cfg });
  }

  private applyTheme() {
    const t = this.cfg.theme;
    if (t === "auto") {
      delete document.documentElement.dataset.theme;
    } else {
      document.documentElement.dataset.theme = t;
    }
  }

  private render() {
    const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
    ($("i-sens") as HTMLInputElement).value = String(this.cfg.sens);
    ($("o-sens") as HTMLOutputElement).textContent = this.cfg.sens.toFixed(1) + "×";
    ($("i-scroll") as HTMLInputElement).value = String(this.cfg.scrollSpeed);
    ($("o-scroll") as HTMLOutputElement).textContent = this.cfg.scrollSpeed.toFixed(1) + "×";
    ($("i-natural") as HTMLInputElement).checked = this.cfg.natural;
    ($("i-tap") as HTMLInputElement).checked = this.cfg.tapEnabled;
    ($("i-lp") as HTMLInputElement).value = String(this.cfg.longPressMs);
    ($("o-lp") as HTMLOutputElement).textContent = this.cfg.longPressMs + "ms";
    ($("i-precise") as HTMLInputElement).checked = this.cfg.precise;
    ($("i-g3u") as HTMLSelectElement).value = this.cfg.g3up;
    ($("i-g3lr") as HTMLSelectElement).value = this.cfg.g3lr;
    ($("i-theme") as HTMLSelectElement).value = this.cfg.theme;
  }
}
