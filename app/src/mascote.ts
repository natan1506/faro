// Vigi, a raposinha do Vigia. As três expressões são as imagens fornecidas
// pelo Natan (assets/fox-*.png); o movimento é só CSS sobre elas.
//
// Humores:
//   calm   — balança a cabeça de leve, os olhos seguem o mouse e ela pisca (está vigiando)
//   alert  — olhos arregalados e "!", com um tremor
//   sleep  — olhos fechados, respirando devagar: o daemon não está respondendo
//
// Olhar e piscar (calm e alert, os humores de olhos abertos) usam camadas
// montadas em tempo real, uma vez, a partir das imagens do Natan, sem desenho novo:
//   rosto  — a imagem do humor com os olhos cobertos pela cor do pelo em volta
//   olhos  — os dois olhos recortados da própria imagem, que se deslocam na direção do cursor
//   piscar — o rosto com os olhos fechados da fox-sleep por cima (a fox-sleep é
//            cinza, por isso não serve inteira)
// Até as camadas ficarem prontas (ou se falharem), vale a imagem original.
import "./mascote.css";
import calmSrc from "./assets/fox-calm.png";
import alertSrc from "./assets/fox-alert.png";
import sleepSrc from "./assets/fox-sleep.png";

export type Mood = "calm" | "alert" | "sleep";

const SRC: Record<Mood, string> = { calm: calmSrc, alert: alertSrc, sleep: sleepSrc };

const reduced = window.matchMedia("(prefers-reduced-motion: reduce)");

interface Layers {
  face: string;
  eyes: [string, string];
  blink: string;
}
const layers: Partial<Record<Mood, Layers>> = {};

/** Medidas nos PNGs de 230×230. Trocou as imagens? Ajuste aqui. */
const SIZE = 230;
/** Olhos abertos de cada humor: centros e raio (o raio cobre o olho, o brilho e a borda suavizada). */
const OPEN_EYES: Partial<Record<Mood, { r: number; at: { x: number; y: number }[] }>> = {
  calm: {
    r: 16,
    at: [
      { x: 84.5, y: 140 },
      { x: 145.5, y: 139.5 },
    ],
  },
  alert: {
    r: 17,
    at: [
      { x: 84.5, y: 140 },
      { x: 144.5, y: 140 },
    ],
  },
};
const CLOSED_EYES = [
  { x0: 64, x1: 102, y0: 134, y1: 153, cx: 83, cy: 143.5 },
  { x0: 128, x1: 166, y0: 135, y1: 153, cx: 147, cy: 144 },
];
/** Quanto os olhos andam na direção do mouse (em pixels do PNG de 230). */
const EYE_TRAVEL = { x: 16, y: 10 };
/** Quanto a cabeça acompanha (graus): virar para os lados, para cima/baixo e inclinar. */
const HEAD = { turn: 8, nod: 6, tilt: 3 };

/** Cria uma raposinha; troque o humor depois com `setMood`. */
export function fox(mood: Mood = "calm", size = 22): HTMLSpanElement {
  const span = document.createElement("span");
  span.className = "fox-wrap";
  span.style.width = span.style.height = `${size}px`;
  // A cabeça leva a animação do humor; dentro dela, as camadas. Piscar e olhar
  // não trocam `src` (trocar deixava um quadro vazio enquanto o WebKit
  // decodificava): só mostram, escondem ou deslocam camadas já carregadas.
  const head = document.createElement("span");
  head.className = "fox-head";
  const img = layer("fox");
  const eyeL = layer("fox-eye");
  const eyeR = layer("fox-eye");
  const lid = layer("fox-blink");
  head.append(img, eyeL, eyeR, lid);
  span.append(head);
  setMood(span, mood);
  return span;
}

function layer(cls: string): HTMLImageElement {
  const img = document.createElement("img");
  img.className = cls;
  img.alt = "";
  img.draggable = false;
  return img;
}

export function setMood(el: HTMLElement, mood: Mood) {
  const l = layers[mood];
  if (el.dataset.mood === mood && (el.dataset.layers === "1") === !!l) return;
  el.dataset.mood = mood;
  const img = el.querySelector<HTMLImageElement>("img.fox");
  if (img) img.src = l ? l.face : SRC[mood];
  if (l) {
    const eyes = el.querySelectorAll<HTMLImageElement>("img.fox-eye");
    eyes[0].src = l.eyes[0];
    eyes[1].src = l.eyes[1];
    el.querySelector<HTMLImageElement>("img.fox-blink")!.src = l.blink;
    el.dataset.layers = "1";
  } else {
    delete el.dataset.layers;
  }
}

// ---------------------------------------------------------------------------
// Montagem das camadas

function loadImage(src: string): Promise<HTMLImageElement> {
  return new Promise((ok, fail) => {
    const img = new Image();
    img.onload = () => ok(img);
    img.onerror = fail;
    img.src = src;
  });
}

function canvas(w: number, h: number): [HTMLCanvasElement, CanvasRenderingContext2D] {
  const c = document.createElement("canvas");
  c.width = w;
  c.height = h;
  return [c, c.getContext("2d")!];
}

function pixels(img: HTMLImageElement): ImageData {
  const [, ctx] = canvas(img.width, img.height);
  ctx.drawImage(img, 0, 0);
  return ctx.getImageData(0, 0, img.width, img.height);
}

function toUrl(data: ImageData): string {
  const [c, ctx] = canvas(data.width, data.height);
  ctx.putImageData(data, 0, 0);
  return c.toDataURL("image/png");
}

/** Carrega e tenta decodificar antes de usar; `decode()` pode nunca responder com a janela escondida. */
async function decoded(src: string): Promise<string> {
  const img = await loadImage(src);
  await Promise.race([img.decode().catch(() => {}), new Promise((ok) => setTimeout(ok, 300))]);
  return src;
}

async function buildLayers(mood: Mood): Promise<Layers> {
  const spec = OPEN_EYES[mood];
  if (!spec) throw new Error(`sem olhos abertos em ${mood}`);
  const EYE_R = spec.r;
  const [base, sleep] = await Promise.all([loadImage(SRC[mood]), loadImage(sleepSrc)]);
  if (base.width !== SIZE || sleep.width !== SIZE) throw new Error("imagens com outro tamanho");
  const original = pixels(base).data;
  const faceData = pixels(base);
  const d = faceData.data;
  const at = (x: number, y: number) => (y * SIZE + x) * 4;

  // Rosto: em cada olho, um círculo preenchido coluna a coluna, interpolando o pelo de cima e o de baixo.
  for (const e of spec.at) {
    for (let x = Math.floor(e.x - EYE_R); x <= Math.ceil(e.x + EYE_R); x++) {
      const half = Math.sqrt(Math.max(0, EYE_R * EYE_R - (x - e.x) ** 2));
      if (half <= 0) continue;
      const y0 = Math.floor(e.y - half) - 1;
      const y1 = Math.ceil(e.y + half) + 1;
      const top = d.slice(at(x, y0), at(x, y0) + 4);
      const bottom = d.slice(at(x, y1), at(x, y1) + 4);
      for (let y = y0 + 1; y < y1; y++) {
        const t = (y - y0) / (y1 - y0);
        for (let k = 0; k < 4; k++) d[at(x, y) + k] = Math.round(top[k] * (1 - t) + bottom[k] * t);
      }
    }
  }

  // Olhos: o que mudou entre o original e o rosto, recortado em camadas do tamanho da imagem.
  const eyes = spec.at.map((e) => {
    const out = new ImageData(SIZE, SIZE);
    for (let y = Math.floor(e.y - EYE_R); y <= Math.ceil(e.y + EYE_R); y++) {
      for (let x = Math.floor(e.x - EYE_R); x <= Math.ceil(e.x + EYE_R); x++) {
        const i = at(x, y);
        const diff = Math.abs(original[i] - d[i]) + Math.abs(original[i + 1] - d[i + 1]) + Math.abs(original[i + 2] - d[i + 2]);
        const alpha = Math.min(1, diff / 90);
        if (alpha <= 0) continue;
        out.data[i] = original[i];
        out.data[i + 1] = original[i + 1];
        out.data[i + 2] = original[i + 2];
        out.data[i + 3] = Math.round(255 * alpha);
      }
    }
    return toUrl(out);
  }) as [string, string];

  // Piscar: o rosto com os traços dos olhos fechados da fox-sleep, centrados nos olhos abertos.
  const blinkData = new ImageData(new Uint8ClampedArray(d), SIZE, SIZE);
  const b = blinkData.data;
  const s = pixels(sleep).data;
  const SCALE = 0.85;
  const FUR = 450; // soma RGB do pelo da fox-sleep em volta dos olhos
  const INK = 200; // soma RGB do traço
  CLOSED_EYES.forEach((c, n) => {
    const e = spec.at[n];
    for (let y = c.y0; y <= c.y1; y++) {
      for (let x = c.x0; x <= c.x1; x++) {
        const i = at(x, y);
        const alpha = Math.min(1, Math.max(0, (FUR - (s[i] + s[i + 1] + s[i + 2])) / (FUR - INK)));
        if (alpha <= 0) continue;
        const j = at(Math.round(e.x + (x - c.cx) * SCALE), Math.round(e.y + 1 + (y - c.cy) * SCALE));
        for (let k = 0; k < 3; k++) b[j + k] = Math.round(b[j + k] * (1 - alpha) + s[i + k] * alpha);
      }
    }
  });

  const [face, eyeL, eyeR, blink] = await Promise.all(
    [toUrl(faceData), eyes[0], eyes[1], toUrl(blinkData)].map(decoded),
  );
  return { face, eyes: [eyeL, eyeR], blink };
}

// ---------------------------------------------------------------------------
// Vida: olhar e piscar

function blinkOnce(ms = 140) {
  const wraps = document.querySelectorAll<HTMLElement>(".fox-wrap[data-layers]");
  for (const w of wraps) w.dataset.blink = "1";
  window.setTimeout(() => {
    for (const w of wraps) delete w.dataset.blink;
  }, ms);
}

/**
 * Monta as camadas e começa a viver: pisca a cada 2,5 a 6 s (às vezes duas
 * seguidas) e, com o mouse parado, olha em volta.
 * Sem "reduzir movimento"; se a montagem falhar, a raposinha fica como a imagem original.
 */
export function startLife() {
  if (reduced.matches) return;
  Promise.all((["calm", "alert"] as const).map(async (m) => [m, await buildLayers(m)] as const))
    .then((built) => {
      for (const [m, l] of built) layers[m] = l;
      for (const wrap of document.querySelectorAll<HTMLElement>(".fox-wrap")) {
        setMood(wrap, (wrap.dataset.mood ?? "calm") as Mood);
      }
      const next = () => {
        window.setTimeout(() => {
          blinkOnce();
          if (Math.random() < 0.2) window.setTimeout(() => blinkOnce(), 260);
          next();
        }, 2500 + Math.random() * 3500);
      };
      next();
      startGlancing();
    })
    .catch(() => {
      // sem as camadas a raposinha só não olha nem pisca
    });
}

// Olhar: os olhos seguem o cursor e a cabeça acompanha de leve, com atraso.
// Com o mouse parado, ela dá umas olhadas para os lados.

let cursor: { x: number; y: number } | null = null;
let lastMove = 0;
let glancing = false;

/**
 * Olha na direção (nx, ny), cada um entre -1 e 1. Os olhos vão na frente,
 * rápidos; a cabeça vem atrás, devagar e só um pouco (vira, inclina e se
 * desloca de leve), como um movimento de verdade. Os tempos ficam no CSS.
 */
function aim(wrap: HTMLElement, nx: number, ny: number) {
  const size = parseFloat(wrap.style.width) || wrap.getBoundingClientRect().width || 22;
  wrap.style.setProperty("--eye-x", `${((nx * EYE_TRAVEL.x * size) / SIZE).toFixed(2)}px`);
  wrap.style.setProperty("--eye-y", `${((ny * EYE_TRAVEL.y * size) / SIZE).toFixed(2)}px`);
  wrap.style.setProperty("--head-x", `${(nx * size * 0.02).toFixed(2)}px`);
  wrap.style.setProperty("--head-y", `${(ny * size * 0.015).toFixed(2)}px`);
  wrap.style.setProperty("--head-turn", `${(nx * HEAD.turn).toFixed(1)}deg`);
  wrap.style.setProperty("--head-nod", `${(-ny * HEAD.nod).toFixed(1)}deg`);
  wrap.style.setProperty("--head-tilt", `${(nx * HEAD.tilt).toFixed(1)}deg`);
}

/** Olha para um ponto da janela (pixels CSS); sem ponto, olha para a frente. */
function lookAtPoint(p: { x: number; y: number } | null) {
  for (const wrap of document.querySelectorAll<HTMLElement>(".fox-wrap")) {
    // dormindo não olha para nada
    if (!p || wrap.dataset.mood === "sleep") {
      aim(wrap, 0, 0);
      continue;
    }
    const r = wrap.getBoundingClientRect();
    if (r.width === 0) continue;
    const dx = p.x - (r.left + r.width / 2);
    const dy = p.y - (r.top + r.height / 2);
    const dist = Math.hypot(dx, dy) || 1;
    const reach = Math.min(1, dist / 120); // perto: olha pouco; longe: vai até o canto do olho
    aim(wrap, (dx / dist) * reach, (dy / dist) * reach);
  }
}

/** Olhada rápida para um lado qualquer, e de volta para o mouse. */
function glance() {
  glancing = true;
  const angle = Math.random() * Math.PI * 2;
  // mais para os lados do que para cima e para baixo, como quem confere o ambiente
  const nx = Math.cos(angle);
  const ny = Math.sin(angle) * 0.6;
  for (const wrap of document.querySelectorAll<HTMLElement>(".fox-wrap[data-layers]")) aim(wrap, nx, ny);
  if (Math.random() < 0.3) window.setTimeout(() => blinkOnce(), 120);
  window.setTimeout(() => {
    glancing = false;
    lookAtPoint(cursor);
  }, 600 + Math.random() * 600);
}

/** Com o mouse parado há uns segundos, de vez em quando ela olha em volta. */
function startGlancing() {
  window.setInterval(() => {
    if (glancing || Date.now() - lastMove < 3500) return;
    if (Math.random() < 0.18) glance();
  }, 500);
}

/** Posição do cursor em pixels CSS da janela. */
export function lookAt(x: number, y: number) {
  if (reduced.matches) return;
  cursor = { x, y };
  lastMove = Date.now();
  if (!glancing) lookAtPoint(cursor);
}
