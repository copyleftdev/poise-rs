#!/usr/bin/env node

// Renders the isometric control-loop diagram published by the site, the README,
// and the architecture chapter.
//
// The drawing is generated rather than hand-authored because its geometry is
// arithmetic: every solid sits on a fixed isometric grid, and a hand-placed
// polygon that is two pixels off the projection reads as a mistake without
// being one. Regenerating is the only supported way to change it:
//
//     node scripts/render-control-loop.mjs
//
// Two encodings carry the meaning. The SHAPE of a solid says what kind of thing
// it is -- a drum holds mutable state, a stack is an immutable revision over
// older ones, a plate carries borrowed candidates, the hexagonal prism is the
// only solid that turns a slice into an index, instruments measure, and dashed
// silhouettes are optional or outside the library. The COLOR says which crate
// owns it. Both are stated in the caption wherever the image is used, because a
// legend that lives only in one page stops being true in the others.
//
// Grid space: gx runs east-down-right, gy runs west-down-left, z lifts.

import { writeFileSync } from "node:fs";
import { resolve } from "node:path";

const TW = 110; // tile width
const TH = 55; // tile height (2:1 dimetric, the isometric convention)
const P = 6; // grid pitch between solids

const iso = (gx, gy, z = 0) => [(gx - gy) * (TW / 2), (gx + gy) * (TH / 2) - z];
const round = (n) => Math.round(n * 100) / 100;
const fmt = ([x, y]) => `${round(x)},${round(y)}`;

// Every drawn point goes through mark() so the viewBox fits the content
// rather than the grid it sits on.
const xs = [];
const ys = [];
const mark = ([x, y]) => {
  xs.push(x);
  ys.push(y);
  return [x, y];
};
const markBox = (x, y, w, h) => {
  mark([x, y]);
  mark([x + w, y + h]);
};

// ---------------------------------------------------------------- palette
const CRATE = {
  core: {
    name: "poise-core",
    top: "oklch(70% 0.152 112)",
    left: "oklch(50% 0.118 112)",
    right: "oklch(39% 0.094 112)",
  },
  discovery: {
    name: "poise-discovery",
    top: "oklch(66% 0.105 190)",
    left: "oklch(47% 0.085 190)",
    right: "oklch(37% 0.068 190)",
  },
  health: {
    name: "poise-health",
    top: "oklch(66% 0.152 38)",
    left: "oklch(48% 0.126 38)",
    right: "oklch(38% 0.1 38)",
  },
  tower: {
    name: "poise-tower",
    top: "oklch(70% 0.058 245)",
    left: "oklch(51% 0.048 245)",
    right: "oklch(40% 0.038 245)",
  },
  tokio: {
    name: "poise-tokio",
    top: "oklch(60% 0.012 145)",
    left: "oklch(45% 0.011 145)",
    right: "oklch(35% 0.009 145)",
  },
  observe: {
    name: "poise-observe",
    top: "oklch(80% 0.032 88)",
    left: "oklch(59% 0.028 88)",
    right: "oklch(46% 0.022 88)",
  },
  external: {
    name: "outside Poise",
    label: "oklch(68% 0.012 145)",
    top: "oklch(45% 0.012 145)",
    left: "oklch(34% 0.011 145)",
    right: "oklch(27% 0.009 145)",
  },
};

const EDGE = {
  membership: "oklch(72% 0.105 190)",
  request: "oklch(88% 0.03 110)",
  feedback: "oklch(66% 0.145 38)",
  adapter: "oklch(62% 0.012 145)",
};

const MUTED = {
  top: "oklch(34% 0.01 145)",
  left: "oklch(28% 0.01 145)",
  right: "oklch(23% 0.008 145)",
};

const DOT = "&#183;"; // middle dot, escaped so the output stays ASCII

// ---------------------------------------------------------------- solids
// A prism over any convex plan polygon. Side faces are drawn only where they
// face the viewer, shaded left or right by the direction they point.
function prism(plan, h, cols, opts = {}) {
  const base = opts.base ?? 0;
  const cgx = plan.reduce((a, q) => a + q[0], 0) / plan.length;
  const cgy = plan.reduce((a, q) => a + q[1], 0) / plan.length;
  const top = plan.map(([gx, gy]) => mark(iso(gx, gy, base + h)));
  const bottom = plan.map(([gx, gy]) => mark(iso(gx, gy, base)));
  const sides = [];
  for (let i = 0; i < plan.length; i += 1) {
    const j = (i + 1) % plan.length;
    const mgx = (plan[i][0] + plan[j][0]) / 2;
    const mgy = (plan[i][1] + plan[j][1]) / 2;
    const nx = mgx - cgx;
    const ny = mgy - cgy;
    if (nx + ny <= 0.0001) continue; // back face
    sides.push({
      depth: mgx + mgy,
      fill: ny > nx ? cols.left : cols.right,
      pts: [top[i], top[j], bottom[j], bottom[i]],
    });
  }
  const faces = sides
    .sort((a, b) => a.depth - b.depth)
    .map((s) => `    <polygon points="${s.pts.map(fmt).join(" ")}" fill="${s.fill}"/>`)
    .join("\n");
  const stroke = opts.dash
    ? ` class="face-top" stroke-dasharray="7 5"`
    : opts.plain
      ? ""
      : ` class="face-top"`;
  return `${faces}
    <polygon points="${top.map(fmt).join(" ")}" fill="${cols.top}"${stroke}/>`;
}

const square = (gx, gy, half) => [
  [gx - half, gy - half],
  [gx + half, gy - half],
  [gx + half, gy + half],
  [gx - half, gy + half],
];

const polygon = (gx, gy, r, sides, phase = 0) =>
  Array.from({ length: sides }, (_, i) => {
    const a = phase + (i * 2 * Math.PI) / sides;
    return [gx + r * Math.cos(a), gy + r * Math.sin(a)];
  });

// An isometric cylinder: a full bottom disc, a straight body, a lit top disc.
function cylinder(gx, gy, r, h, cols, id) {
  const rx = r * (TW / 2);
  const ry = r * (TH / 2);
  const [cx, cyTop] = iso(gx, gy, h);
  const cyBottom = cyTop + h;
  markBox(cx - rx, cyTop - ry, rx * 2, h + ry * 2);
  return `    <ellipse cx="${round(cx)}" cy="${round(cyBottom)}" rx="${round(rx)}" ry="${round(ry)}" fill="${cols.right}"/>
    <rect x="${round(cx - rx)}" y="${round(cyTop)}" width="${round(rx * 2)}" height="${round(h)}" fill="url(#${id})"/>
    <ellipse cx="${round(cx)}" cy="${round(cyTop)}" rx="${round(rx)}" ry="${round(ry)}" fill="${cols.top}" class="face-top"/>`;
}

const shadow = (gx, gy, half) =>
  `    <polygon class="contact" points="${square(gx, gy, half + 0.16)
    .map(([x, y]) => fmt(mark(iso(x, y + 0.18))))
    .join(" ")}"/>`;

const discShadow = (gx, gy, r) => {
  const [cx, cy] = iso(gx, gy, 0);
  return `    <ellipse class="contact" cx="${round(cx)}" cy="${round(cy + 10)}" rx="${round(r * (TW / 2) + 10)}" ry="${round(r * (TH / 2) + 6)}"/>`;
};

// ---------------------------------------------------------------- the board
// Each entry names the solid that describes it; see SHAPES below.
const nodes = [
  {
    gx: 0,
    gy: 0,
    crate: "tokio",
    shape: "shim",
    title: "Tokio adapter",
    sub: [`timers ${DOT} revision waits`],
  },
  {
    gx: P,
    gy: 0,
    crate: "discovery",
    shape: "store",
    title: "Directory",
    sub: ["single writer, staged"],
  },
  {
    gx: 2 * P,
    gy: 0,
    crate: "discovery",
    shape: "stack",
    title: "Snapshot",
    sub: [`immutable ${DOT} revision n`],
  },
  {
    gx: P,
    gy: P,
    crate: "health",
    shape: "store",
    title: "Health",
    sub: ["probes, circuits, ejection"],
  },
  {
    gx: 2 * P,
    gy: P,
    crate: "core",
    shape: "view",
    title: "Candidate view",
    sub: ["one excluded, with a reason"],
  },
  {
    gx: 3 * P,
    gy: P,
    crate: "core",
    shape: "chooser",
    title: "Policy",
    sub: ["returns an index"],
  },
  {
    gx: 0,
    gy: 2 * P,
    crate: "observe",
    shape: "beacon",
    title: "Observer",
    sub: ["bounded counters"],
  },
  {
    gx: P,
    gy: 2 * P,
    crate: "health",
    shape: "window",
    title: "Outcome window",
    sub: ["bounded rolling history"],
  },
  {
    gx: 2 * P,
    gy: 2 * P,
    crate: "core",
    shape: "gauges",
    title: "Load trackers",
    sub: [`in-flight ${DOT} peak EWMA`],
  },
  {
    gx: 3 * P,
    gy: 2 * P,
    crate: "tower",
    shape: "service",
    title: "Tower Balance",
    sub: ["readiness retained"],
  },
  {
    gx: 4 * P,
    gy: 2 * P,
    crate: "external",
    shape: "rack",
    title: "Endpoint pool",
    sub: ["the services you dispatch to"],
  },
];

const LANE = 17; // the feedback return lane, south of every solid

const paths = [
  {
    kind: "membership",
    pts: [[P, -4.6], [P, -1.4]],
    label: { lines: ["apply batch"], seg: 0, dx: -10, dy: -6, anchor: "end" },
  },
  {
    kind: "membership",
    pts: [[P + 1.4, 0], [2 * P - 1.4, 0]],
    label: {
      lines: ["revision n+1", "atomic swap"],
      seg: 0,
      dx: 0,
      dy: -22,
      anchor: "middle",
    },
  },
  {
    kind: "membership",
    pts: [[2 * P, 1.4], [2 * P, P - 1.4]],
    label: {
      lines: ["coherent slice", "of candidates"],
      seg: 0,
      dx: 16,
      dy: 0,
      anchor: "start",
    },
  },
  {
    kind: "feedback",
    pts: [[P + 1.4, P], [2 * P - 1.4, P]],
    label: {
      lines: [`probe class ${DOT}`, "circuit permit"],
      seg: 0,
      dx: 0,
      dy: -22,
      anchor: "middle",
    },
  },
  {
    kind: "request",
    pts: [[2 * P + 1.4, P], [3 * P - 1.5, P]],
    label: {
      lines: ["eligible slice", "+ request key"],
      seg: 0,
      dx: 0,
      dy: -22,
      anchor: "middle",
    },
  },
  {
    kind: "request",
    pts: [[3 * P, P + 1.5], [3 * P, 2 * P - 1.4]],
    label: {
      lines: ["an index,", "not a dispatch"],
      seg: 0,
      dx: 16,
      dy: 0,
      anchor: "start",
    },
  },
  {
    kind: "request",
    pts: [[3 * P + 1.4, 2 * P], [4 * P - 1.6, 2 * P]],
    label: {
      lines: ["one readiness", "permit consumed"],
      seg: 0,
      dx: 0,
      dy: -22,
      anchor: "middle",
    },
  },
  {
    kind: "feedback",
    pts: [
      [4 * P, 2 * P + 1.5],
      [4 * P, LANE],
      [2 * P, LANE],
      [2 * P, 2 * P + 1.4],
    ],
    label: {
      lines: [`completed attempt ${DOT} latency, class, cancellation`],
      seg: 1,
      dx: 0,
      dy: 30,
      anchor: "middle",
    },
  },
  {
    kind: "feedback",
    pts: [[2 * P, LANE], [P, LANE], [P, 2 * P + 1.4]],
    label: {
      lines: ["cancellation", "is not failure"],
      seg: 0,
      dx: 0,
      dy: 30,
      anchor: "middle",
    },
  },
  {
    kind: "feedback",
    pts: [[P, 2 * P - 1.4], [P, P + 1.4]],
    label: {
      lines: [`consecutive results ${DOT}`, "circuit epoch"],
      seg: 0,
      dx: -16,
      dy: 0,
      anchor: "end",
    },
  },
  {
    kind: "feedback",
    pts: [[P - 1.4, 2 * P], [1.4, 2 * P]],
    label: {
      lines: ["no endpoint labels"],
      seg: 0,
      dx: -120,
      dy: 30,
      anchor: "middle",
    },
  },
  {
    kind: "feedback",
    pts: [[2 * P, 2 * P - 1.4], [2 * P, P + 1.4]],
    label: {
      lines: ["sampled load", "metric"],
      seg: 0,
      dx: -16,
      dy: 0,
      anchor: "end",
    },
  },
  {
    kind: "adapter",
    pts: [[1.4, 0], [P - 1.4, 0]],
    label: { lines: ["revision wait"], seg: 0, dx: 0, dy: 28, anchor: "middle" },
  },
  {
    kind: "adapter",
    pts: [[0, 1.4], [0, P], [P - 1.4, P]],
    label: {
      lines: [`due probe ${DOT}`, "timeout"],
      seg: 1,
      dx: 0,
      dy: 30,
      anchor: "middle",
    },
  },
];

// ---------------------------------------------------------------- shapes
// Every builder returns the solid's SVG plus the point its label hangs from.
const SHAPES = {
  // A service: the plain cube.
  service: (n, c) => ({
    svg: shadow(n.gx, n.gy, 1.25) + "\n" + prism(square(n.gx, n.gy, 1.25), 30, c),
    anchor: iso(n.gx - 1.25, n.gy - 1.25, 30),
  }),

  // A store you write to: a drum of mutable state.
  store: (n, c) => ({
    svg:
      discShadow(n.gx, n.gy, 1.3) +
      "\n" +
      cylinder(n.gx, n.gy, 1.3, 34, c, `cyl-${n.crate}`),
    anchor: [iso(n.gx, n.gy, 34)[0] - 1.3 * (TW / 2), iso(n.gx, n.gy, 34)[1] - 1.3 * (TH / 2)],
  }),

  // Immutable revisions: plates stacked, only the newest one lit.
  stack: (n, c) => {
    const older = { top: c.left, left: c.right, right: c.right };
    return {
      svg: [
        shadow(n.gx, n.gy, 1.25),
        prism(square(n.gx, n.gy, 1.25), 9, older, { base: 0 }),
        prism(square(n.gx, n.gy, 1.25), 9, older, { base: 15 }),
        prism(square(n.gx, n.gy, 1.25), 11, c, { base: 30 }),
      ].join("\n"),
      anchor: iso(n.gx - 1.25, n.gy - 1.25, 41),
    };
  },

  // A borrowed view: a thin plate carrying four candidates, one excluded.
  view: (n, c) => {
    const chips = [
      [-0.58, -0.58],
      [0.58, -0.58],
      [-0.58, 0.58],
      [0.58, 0.58],
    ]
      .map(([dx, dy], i) => ({
        depth: dx + dy,
        svg: prism(square(n.gx + dx, n.gy + dy, 0.3), i === 3 ? 8 : 18, i === 3 ? MUTED : c, {
          base: 10,
          dash: i === 3,
        }),
      }))
      .sort((a, b) => a.depth - b.depth)
      .map((s) => s.svg)
      .join("\n");
    return {
      svg: [
        shadow(n.gx, n.gy, 1.35),
        prism(square(n.gx, n.gy, 1.35), 10, { top: c.left, left: c.left, right: c.right }),
        chips,
      ].join("\n"),
      anchor: iso(n.gx - 1.35, n.gy - 1.35, 28),
    };
  },

  // The chooser: a hexagonal prism, the one solid that turns a slice into an index.
  chooser: (n, c) => ({
    svg:
      shadow(n.gx, n.gy, 1.2) +
      "\n" +
      prism(polygon(n.gx, n.gy, 1.45, 6, Math.PI / 6), 34, c),
    anchor: iso(n.gx - 1.3, n.gy - 1.3, 34),
  }),

  // Instruments: three columns at different heights.
  gauges: (n, c) => {
    const bars = [
      [-0.72, 46],
      [0, 26],
      [0.72, 36],
    ]
      .map(([d, h]) => ({
        depth: d,
        svg: prism(square(n.gx, n.gy + d, 0.28), h, c, { base: 10 }),
      }))
      .sort((a, b) => a.depth - b.depth)
      .map((s) => s.svg)
      .join("\n");
    return {
      svg: [
        shadow(n.gx, n.gy, 1.3),
        prism(square(n.gx, n.gy, 1.3), 10, { top: c.left, left: c.left, right: c.right }),
        bars,
      ].join("\n"),
      anchor: iso(n.gx - 1.3, n.gy - 1.3, 56),
    };
  },

  // A beacon: a low plate under a mast, reporting outward.
  beacon: (n, c) => ({
    svg: [
      shadow(n.gx, n.gy, 1.25),
      prism(square(n.gx, n.gy, 1.25), 10, { top: c.left, left: c.left, right: c.right }),
      prism(square(n.gx, n.gy, 0.16), 44, c, { base: 10 }),
      prism(square(n.gx, n.gy, 0.5), 10, c, { base: 54 }),
    ].join("\n"),
    anchor: iso(n.gx - 1.25, n.gy - 1.25, 64),
  }),

  // An optional shim: dashed, and barely off the ground.
  shim: (n, c) => ({
    svg:
      shadow(n.gx, n.gy, 1.25) +
      "\n" +
      prism(square(n.gx, n.gy, 1.25), 12, c, { dash: true }),
    anchor: iso(n.gx - 1.25, n.gy - 1.25, 12),
  }),

  // A rack of services, outside the library: dashed pad, three endpoints.
  rack: (n, c) => {
    const cubes = [-0.95, 0, 0.95]
      .map((d) => ({
        depth: d,
        svg: prism(square(n.gx, n.gy + d, 0.38), 26, {
          top: "oklch(66% 0.016 145)",
          left: "oklch(44% 0.014 145)",
          right: "oklch(35% 0.012 145)",
        }, { base: 12, plain: true }),
      }))
      .sort((a, b) => a.depth - b.depth)
      .map((s) => s.svg)
      .join("\n");
    return {
      svg: [
        shadow(n.gx, n.gy, 1.55),
        prism(square(n.gx, n.gy, 1.55), 12, c, { dash: true }),
        cubes,
      ].join("\n"),
      anchor: iso(n.gx - 1.55, n.gy - 1.55, 38),
    };
  },
};

// Bounded history: a plate of samples, the oldest one leaving the window.
SHAPES.window = (n, c) => {
  const tiles = [-0.92, -0.46, 0, 0.46, 0.92]
    .map((d, i) => ({
      depth: d,
      svg: prism(square(n.gx + d, n.gy, 0.17), i === 0 ? 6 : 15, i === 0 ? MUTED : c, {
        base: 10,
        dash: i === 0,
      }),
    }))
    .sort((a, b) => a.depth - b.depth)
    .map((s) => s.svg)
    .join("\n");
  return {
    svg: [
      shadow(n.gx, n.gy, 1.35),
      prism(square(n.gx, n.gy, 1.35), 10, { top: c.left, left: c.left, right: c.right }),
      tiles,
    ].join("\n"),
    anchor: iso(n.gx - 1.35, n.gy - 1.35, 25),
  };
};

function solid(node) {
  const c = CRATE[node.crate];
  const { svg, anchor } = SHAPES[node.shape](node, c);
  return {
    svg: `  <g class="blk" data-crate="${node.crate}">\n${svg}\n  </g>`,
    anchor,
  };
}

// ---------------------------------------------------------------- labels
const plain = (s) => s.replace(/&#\d+;/g, "x");

function pill(anchor, crate, title, sub) {
  const [x, y] = anchor;
  const crateName = CRATE[crate].name;
  const w = Math.min(
    308,
    Math.max(
      title.length * 12.6,
      Math.max(...sub.map((s) => plain(s).length)) * 8.8,
      crateName.length * 8.8,
    ) + 38,
  );
  const boxH = 34 + sub.length * 19 + 17;
  const top = y - 32 - boxH;
  markBox(x - w / 2, top, w, boxH);
  const subLines = sub
    .map((s, i) => `<tspan x="${round(x)}" dy="${i ? 19 : 0}">${s}</tspan>`)
    .join("");
  return `  <g class="pill" data-crate="${crate}">
    <line x1="${round(x)}" y1="${round(y)}" x2="${round(x)}" y2="${round(top + boxH)}" class="pill-stem"/>
    <rect x="${round(x - w / 2)}" y="${round(top)}" width="${round(w)}" height="${boxH}" rx="8" class="pill-bg"/>
    <text x="${round(x)}" y="${round(top + 24)}" text-anchor="middle" class="pill-title">${title}</text>
    <text x="${round(x)}" y="${round(top + 45)}" text-anchor="middle" class="pill-sub">${subLines}</text>
    <text x="${round(x)}" y="${round(top + boxH - 10)}" text-anchor="middle" class="pill-crate" fill="${CRATE[crate].label ?? CRATE[crate].top}">${crateName}</text>
  </g>`;
}

function sourceTag() {
  const [x, y] = iso(P, -5.4);
  const w = 336;
  markBox(x - w / 2, y - 32, w, 58);
  return `  <g class="pill" data-crate="discovery">
    <rect x="${round(x - w / 2)}" y="${round(y - 32)}" width="${w}" height="58" rx="8" class="pill-bg"/>
    <text x="${round(x)}" y="${round(y - 9)}" text-anchor="middle" class="pill-title">DNS ${DOT} xDS ${DOT} config ${DOT} k8s</text>
    <text x="${round(x)}" y="${round(y + 13)}" text-anchor="middle" class="pill-sub">whatever knows the membership</text>
  </g>`;
}

const pathEl = (pth) =>
  `  <path class="edge edge-${pth.kind}" d="${pth.pts
    .map((q, i) => `${i ? "L" : "M"}${fmt(mark(iso(...q)))}`)
    .join(" ")}" marker-end="url(#arw-${pth.kind})"/>`;

function pathLabel(pth) {
  const { lines, seg, dx, dy, anchor } = pth.label;
  const a = iso(...pth.pts[seg]);
  const b = iso(...pth.pts[seg + 1]);
  const x = (a[0] + b[0]) / 2 + dx;
  const y = (a[1] + b[1]) / 2 + dy;
  const boxW = Math.max(...lines.map((l) => plain(l).length)) * 10.2 + 22;
  const boxH = lines.length * 21 + 12;
  const bx =
    anchor === "middle" ? x - boxW / 2 : anchor === "end" ? x - boxW + 11 : x - 11;
  const by = y - 18;
  markBox(bx, by, boxW, boxH);
  const text = lines
    .map((l, i) => `<tspan x="${round(x)}" dy="${i ? 21 : 0}">${l}</tspan>`)
    .join("");
  return `  <g class="edge-label edge-label-${pth.kind}">
    <rect x="${round(bx)}" y="${round(by)}" width="${round(boxW)}" height="${round(boxH)}" rx="6" class="label-bg"/>
    <text x="${round(x)}" y="${round(y)}" text-anchor="${anchor}">${text}</text>
  </g>`;
}

function gridEl() {
  const gxMin = -5;
  const gxMax = 4 * P + 5;
  const gyMin = -8;
  const gyMax = LANE + 5;
  const out = [];
  for (let gx = gxMin; gx <= gxMax + 0.01; gx += 1.5) {
    const major = Math.abs(gx % P) < 0.01;
    const [x1, y1] = iso(gx, gyMin);
    const [x2, y2] = iso(gx, gyMax);
    out.push(
      `    <line class="${major ? "g-major" : "g-minor"}" x1="${round(x1)}" y1="${round(y1)}" x2="${round(x2)}" y2="${round(y2)}"/>`,
    );
  }
  for (let gy = gyMin; gy <= gyMax + 0.01; gy += 1.5) {
    const major = Math.abs(gy % P) < 0.01;
    const [x1, y1] = iso(gxMin, gy);
    const [x2, y2] = iso(gxMax, gy);
    out.push(
      `    <line class="${major ? "g-major" : "g-minor"}" x1="${round(x1)}" y1="${round(y1)}" x2="${round(x2)}" y2="${round(y2)}"/>`,
    );
  }
  return `  <g class="grid" aria-hidden="true">\n${out.join("\n")}\n  </g>`;
}

// ---------------------------------------------------------------- assemble
const byDepth = [...nodes].sort((a, b) => a.gx + a.gy - (b.gx + b.gy));
const built = byDepth.map((n) => ({ node: n, ...solid(n) }));

const parts = [];
parts.push(gridEl());
parts.push(`  <g class="edges">\n${paths.map(pathEl).join("\n")}\n  </g>`);
parts.push(built.map((b) => b.svg).join("\n"));
parts.push(`  <g class="edge-labels">\n${paths.map(pathLabel).join("\n")}\n  </g>`);
parts.push(sourceTag());
parts.push(
  built.map((b) => pill(b.anchor, b.node.crate, b.node.title, b.node.sub)).join("\n"),
);

const pad = 34;
const minX = Math.min(...xs) - pad;
const maxX = Math.max(...xs) + pad;
const minY = Math.min(...ys) - pad;
const maxY = Math.max(...ys) + pad;
const w = maxX - minX;
const h = maxY - minY;

const marker = (id, color) =>
  `    <marker id="arw-${id}" viewBox="0 0 12 12" refX="10" refY="6" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
      <path d="M1 1 L11 6 L1 11 z" fill="${color}"/>
    </marker>`;

// Cylinder bodies are shaded by a gradient rather than flat faces.
const cylGradient = (key) => {
  const c = CRATE[key];
  return `    <linearGradient id="cyl-${key}" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0" stop-color="${c.right}"/>
      <stop offset="0.34" stop-color="${c.left}"/>
      <stop offset="1" stop-color="${c.right}"/>
    </linearGradient>`;
};


// The image carries its own styles: it is used as a plain <img> in three
// documents, none of which can lend it their CSS.
const STANDALONE_STYLE = `  <style>
    .board-bg { fill: #131a16; }
    .g-minor { stroke: oklch(75% 0.03 145 / 0.05); stroke-width: 1; }
    .g-major { stroke: oklch(75% 0.03 145 / 0.1); stroke-width: 1; }
    .contact { fill: oklch(8% 0.01 145 / 0.5); }
    .face-top { stroke: oklch(96% 0.02 110 / 0.24); stroke-width: 1.4; }
    .edge { fill: none; stroke-width: 3.4; stroke-linecap: round; stroke-linejoin: round; }
    .edge-membership { stroke: ${EDGE.membership}; }
    .edge-request { stroke: ${EDGE.request}; }
    .edge-feedback { stroke: ${EDGE.feedback}; }
    .edge-adapter { stroke: ${EDGE.adapter}; stroke-width: 2.6; stroke-dasharray: 9 7; }
    .edge-label text { font-family: ui-monospace, "SF Mono", "JetBrains Mono", Menlo, Consolas, monospace; font-size: 17px; fill: oklch(90% 0.02 110 / 0.94); }
    .edge-label-membership text { fill: oklch(84% 0.07 190); }
    .edge-label-feedback text { fill: oklch(82% 0.09 38); }
    .edge-label-adapter text { fill: oklch(74% 0.012 145); }
    .label-bg { fill: oklch(19% 0.016 145 / 0.86); }
    .pill-bg { fill: oklch(14% 0.014 145 / 0.94); stroke: oklch(72% 0.03 145 / 0.26); }
    .pill-stem { stroke: oklch(72% 0.03 145 / 0.34); stroke-width: 1.4; stroke-dasharray: 3 4; }
    .pill-title { font-family: ui-monospace, "SF Mono", "JetBrains Mono", Menlo, Consolas, monospace; font-size: 21px; font-weight: 500; fill: oklch(96% 0.015 110); }
    .pill-sub { font-family: ui-monospace, "SF Mono", "JetBrains Mono", Menlo, Consolas, monospace; font-size: 15px; fill: oklch(74% 0.02 145); }
    .pill-crate { font-family: ui-monospace, "SF Mono", "JetBrains Mono", Menlo, Consolas, monospace; font-size: 13px; }
  </style>
`;

const aria =
  "Isometric map of the Poise control loop. Discovery publishes an immutable snapshot; " +
  "health and load signals narrow it to an eligible candidate slice; a policy returns an index; " +
  "Tower dispatches using a readiness permit it already held; and the classified outcome returns " +
  "along a feedback lane into load trackers, the outcome window, health circuits, and metrics.";

const openTag = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${round(minX)} ${round(minY)} ${round(w)} ${round(h)}" width="${round(w)}" height="${round(h)}" role="img" aria-label="${aria}">`;

const svg = `${openTag}
${STANDALONE_STYLE}  <defs>
${marker("membership", EDGE.membership)}
${marker("request", EDGE.request)}
${marker("feedback", EDGE.feedback)}
${marker("adapter", EDGE.adapter)}
${cylGradient("discovery")}
${cylGradient("health")}
  </defs>
  <rect x="${round(minX)}" y="${round(minY)}" width="${round(w)}" height="${round(h)}" class="board-bg"/>
${parts.join("\n")}
</svg>`;

const output = process.argv[2] ?? resolve(import.meta.dirname, "..", "docs", "assets", "control-loop.svg");
writeFileSync(output, `${svg}\n`);
console.error(`Rendered ${round(w)} x ${round(h)} to ${output}`);
