// 键盘面板（§5.1）：修饰键粘滞、组合键、IME 整句上屏、媒体键、长按退格。

type Send = (obj: object) => void;

const MODS = ["cmd", "opt", "ctrl", "shift"] as const;
const MOD_LABEL: Record<string, string> = { cmd: "⌘", opt: "⌥", ctrl: "⌃", shift: "⇧" };

export class KeyboardCtl {
  private sticky = new Set<string>();
  private imeTimer = 0;
  private composing = false;

  constructor(
    private send: Send,
    private els: {
      keybar: HTMLElement;
      keysPanel: HTMLElement;
      ime: HTMLInputElement;
      onToast: (msg: string) => void;
    },
  ) {
    this.bindMods();
    this.bindKeys();
    this.bindMedia();
    this.bindBackspace();
    this.bindIme();
  }

  private bindMods() {
    this.els.keybar.querySelectorAll<HTMLButtonElement>(".mod").forEach((btn) => {
      btn.addEventListener("click", () => {
        const m = btn.dataset.mod!;
        if (this.sticky.has(m)) {
          this.sticky.delete(m);
          btn.classList.remove("sticky");
        } else {
          this.sticky.add(m);
          btn.classList.add("sticky");
        }
      });
    });
  }

  /** 组合键：粘滞修饰键 + 目标键；发送后清空粘滞（§4.2 combo） */
  private withMods(key: string): boolean {
    if (this.sticky.size === 0) return false;
    this.send({ t: "combo", ks: [...this.sticky, key] });
    this.clearSticky();
    return true;
  }

  private clearSticky() {
    this.sticky.clear();
    this.els.keybar.querySelectorAll(".mod").forEach((b) => b.classList.remove("sticky"));
  }

  private bindKeys() {
    document.querySelectorAll<HTMLButtonElement>("button[data-key]").forEach((btn) => {
      btn.addEventListener("click", () => {
        const k = btn.dataset.key!;
        if (!this.withMods(k)) this.send({ t: "tap", k });
      });
    });
    document.querySelectorAll<HTMLButtonElement>("button[data-char]").forEach((btn) => {
      btn.addEventListener("click", () => {
        const c = btn.dataset.char!;
        if (!this.withMods(c)) this.send({ t: "text", s: c });
      });
    });
  }

  private bindMedia() {
    document.querySelectorAll<HTMLButtonElement>("button[data-media]").forEach((btn) => {
      btn.addEventListener("click", () => {
        this.send({ t: "media", k: btn.dataset.media! });
      });
    });
  }

  private bindBackspace() {
    const btn = document.getElementById("bs-btn") as HTMLButtonElement | null;
    if (!btn) return;
    // data-key 已绑定单击；这里补充长按连发
    let repeat = 0;
    const stop = () => clearInterval(repeat);
    btn.addEventListener("pointerdown", () => {
      repeat = setInterval(() => this.send({ t: "backspace", n: 1 }), 120) as unknown as number;
    });
    for (const t of ["pointerup", "pointercancel", "pointerleave"] as const) {
      btn.addEventListener(t, stop);
    }
  }

  /** IME 整句注入（§5.1）：compositionend 或防抖后的最终文本 */
  private bindIme() {
    const ime = this.els.ime;
    ime.addEventListener("compositionstart", () => {
      this.composing = true;
    });
    ime.addEventListener("compositionend", () => {
      this.composing = false;
      this.flushIme();
    });
    ime.addEventListener("input", () => {
      if (this.composing) return;
      clearTimeout(this.imeTimer);
      this.imeTimer = setTimeout(() => this.flushIme(), 250) as unknown as number;
    });
    ime.addEventListener("keydown", (e) => {
      if (e.isComposing || this.composing) return;
      if (e.key === "Enter") {
        e.preventDefault();
        this.flushIme();
        this.send({ t: "tap", k: "enter" });
      } else if (e.key === "Backspace" && ime.value === "") {
        this.send({ t: "backspace", n: 1 });
      }
    });
  }

  private flushIme() {
    clearTimeout(this.imeTimer);
    const v = this.els.ime.value;
    if (!v) return;
    this.els.ime.value = "";
    this.send({ t: "text", s: v });
  }

  focusIme() {
    this.els.ime.focus();
  }

  blurIme() {
    this.els.ime.blur();
  }

  static label(mod: string): string {
    return MOD_LABEL[mod] ?? mod;
  }

  static get mods(): readonly string[] {
    return MODS;
  }
}
