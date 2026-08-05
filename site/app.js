const FALLBACK_RECORD = {
  schemaVersion: 1,
  provenance: {
    kind: "recorded",
    label: "Recorded local baseline",
    generatedAt: "2026-08-04T19:00:00-07:00",
    revision: "source snapshot",
    sourceUrl: "",
  },
  project: { repository: "", docs: "../docs/architecture.md" },
  verification: {
    status: "passed",
    ordinary: { value: 240, detail: "authored test entry points", status: "passed" },
    property: { value: 3584, laws: 14, casesPerLaw: 256, status: "passed" },
    mutation: { sites: 642, caught: 497, missed: 0, timeout: 23, unviable: 122, status: "passed" },
    loom: { value: 6, status: "passed" },
    msrv: { value: "1.85", status: "passed" },
  },
};

const CAPABILITIES = [
  { id: "selection", name: "Selection", detail: "eligible choice", hue: 112 },
  { id: "affinity", name: "Affinity", detail: "stable placement", hue: 83 },
  { id: "topology", name: "Topology", detail: "priority + locality", hue: 44 },
  { id: "health", name: "Health + load", detail: "feedback control", hue: 24 },
  { id: "discovery", name: "Discovery", detail: "snapshot transition", hue: 168 },
  { id: "observe", name: "Observability", detail: "bounded telemetry", hue: 193 },
  { id: "runtime", name: "Runtime bridges", detail: "Tower + Tokio", hue: 125 },
];

const EVENTS = [
  { id: "ordinary", title: "Ordinary tests", detail: "authored behavioral examples", mode: 0 },
  { id: "property", title: "Property laws", detail: "generated cases", mode: 1 },
  { id: "mutation", title: "Mutation gate", detail: "viable survivors", mode: 2 },
  { id: "loom", title: "Loom models", detail: "scheduler interleavings", mode: 3 },
  { id: "msrv", title: "Compiler floor", detail: "minimum supported Rust", mode: 4 },
];

const reduceMotion = matchMedia("(prefers-reduced-motion: reduce)");
const canvas = document.querySelector("#field");
const context = canvas.getContext?.("2d", { alpha: true, desynchronized: true });
const replayButton = document.querySelector("#replay-button");
const eventButtons = [...document.querySelectorAll("[data-event]")];
const capabilityButtons = [...document.querySelectorAll("[data-capability]")];
const orrery = document.querySelector(".orrery");
const liveDescription = document.querySelector("#live-description");

let record = FALLBACK_RECORD;
let selectedCapability = "all";
let hoveredCapability = null;
let eventIndex = 0;
let playing = !reduceMotion.matches;
let inView = true;
let frameRequest = 0;
let lastFrame = 0;
let eventStartedAt = performance.now();

function formatNumber(value) {
  return new Intl.NumberFormat("en-US").format(value);
}

function formatDate(value) {
  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) return "date unavailable";
  return new Intl.DateTimeFormat("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "numeric",
    minute: "2-digit",
    timeZoneName: "short",
  }).format(date);
}

function evidenceFor(event) {
  const proof = record.verification[event.id];
  if (!proof) return "result unavailable";
  if (event.id === "ordinary") return `${formatNumber(proof.value)} authored test entry points`;
  if (event.id === "property") return `${formatNumber(proof.value)} generated cases`;
  if (event.id === "mutation") return `${formatNumber(proof.missed)} viable survivors / ${formatNumber(proof.sites)} sites`;
  if (event.id === "loom") return `${formatNumber(proof.value)} scheduler models`;
  return `Rust ${proof.value} minimum`;
}

function applyRecord(nextRecord) {
  record = nextRecord;
  const provenance = record.provenance ?? FALLBACK_RECORD.provenance;
  const verification = record.verification ?? FALLBACK_RECORD.verification;
  const isLive = provenance.kind === "ci";
  const generatedTime = new Date(provenance.generatedAt).valueOf();
  const isStale = Number.isFinite(generatedTime) && Date.now() - generatedTime > 14 * 24 * 60 * 60 * 1000;
  const label = provenance.label || (isLive ? "Live CI verification" : "Recorded verification");
  const detail = `${formatDate(provenance.generatedAt)} · ${provenance.revision || "revision unavailable"}`;

  const provenanceLink = document.querySelector("#provenance-title");
  provenanceLink.textContent = label;
  if (provenance.sourceUrl) provenanceLink.href = provenance.sourceUrl;
  else provenanceLink.removeAttribute("href");
  document.querySelector("#provenance-detail").textContent = detail;
  document.querySelector("#run-state").textContent = `${isStale ? "Stale" : isLive ? "CI" : "Recorded"} · ${verification.status || "unknown"} · ${formatDate(provenance.generatedAt)}`;
  document.documentElement.dataset.verification = verification.status || "unknown";
  document.documentElement.dataset.freshness = isStale ? "stale" : "current";

  for (const proof of ["ordinary", "property", "mutation", "loom", "msrv"]) {
    const status = verification[proof]?.status || "unavailable";
    const article = document.querySelector(`[data-proof="${proof}"]`);
    article.dataset.status = status;
    const statusLabel = document.querySelector(`[data-proof-status="${proof}"]`);
    statusLabel.textContent = status;
  }

  const ordinary = verification.ordinary?.value;
  const property = verification.property?.value;
  const mutation = verification.mutation;
  const loom = verification.loom?.value;
  const msrv = verification.msrv?.value;
  if (ordinary != null) document.querySelector('[data-value="ordinary"]').textContent = formatNumber(ordinary);
  if (property != null) document.querySelector('[data-value="property"]').textContent = formatNumber(property);
  if (mutation?.missed != null) {
    document.querySelector('[data-value="mutation"]').textContent = formatNumber(mutation.missed);
    document.querySelector('[data-value="mutation"] + .proof-unit').textContent = `viable survivors / ${formatNumber(mutation.sites)} sites`;
  }
  if (loom != null) document.querySelector('[data-value="loom"]').textContent = formatNumber(loom);
  if (msrv != null) document.querySelector('[data-value="msrv"]').textContent = msrv;

  const repository = record.project?.repository;
  if (repository) {
    document.querySelectorAll("[data-repo-link]").forEach((link) => { link.href = repository; });
  }
  const docs = record.project?.docs;
  if (docs) document.querySelector("[data-docs-link]").href = docs;
  updateEvent(false);
}

async function loadRecord() {
  try {
    const response = await fetch("data/latest.json", { cache: "no-store" });
    if (!response.ok) throw new Error(`verification record returned ${response.status}`);
    const loaded = await response.json();
    if (loaded.schemaVersion !== 1 || !loaded.verification || !loaded.provenance) {
      throw new Error("verification record schema is unsupported");
    }
    applyRecord(loaded);
  } catch (error) {
    console.info("Poise showcase is using its truthful embedded verification baseline.", error);
    applyRecord(FALLBACK_RECORD);
  }
}

class DeterministicRandom {
  constructor(seed) { this.seed = seed >>> 0; }
  next() {
    this.seed = (1664525 * this.seed + 1013904223) >>> 0;
    return this.seed / 4294967296;
  }
}

class Orrery {
  constructor(element, ctx) {
    this.canvas = element;
    this.ctx = ctx;
    this.width = 1;
    this.height = 1;
    this.dpr = 1;
    this.centerX = 0;
    this.centerY = 0;
    this.systemRadius = 200;
    this.particles = [];
    this.nodes = CAPABILITIES.map((capability, index) => ({ ...capability, index, x: 0, y: 0, radius: 24 }));
    this.random = new DeterministicRandom(0x504f4953);
    this.resize = this.resize.bind(this);
    this.onPointerMove = this.onPointerMove.bind(this);
    this.onPointerLeave = this.onPointerLeave.bind(this);
    this.onClick = this.onClick.bind(this);
    this.canvas.addEventListener("pointermove", this.onPointerMove, { passive: true });
    this.canvas.addEventListener("pointerleave", this.onPointerLeave);
    this.canvas.addEventListener("click", this.onClick);
    new ResizeObserver(this.resize).observe(this.canvas);
    this.resize();
  }

  particleBudget() {
    if (reduceMotion.matches) return this.width < 700 ? 84 : 140;
    if (this.width < 580) return 130;
    if (this.width < 960) return 210;
    return 420;
  }

  resize() {
    const rect = this.canvas.getBoundingClientRect();
    this.width = Math.max(1, rect.width);
    this.height = Math.max(1, rect.height);
    this.dpr = Math.min(window.devicePixelRatio || 1, this.width < 700 ? 1.25 : 1.5);
    this.canvas.width = Math.round(this.width * this.dpr);
    this.canvas.height = Math.round(this.height * this.dpr);
    this.ctx.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
    this.centerX = this.width < 900 ? this.width * 0.52 : this.width * 0.73;
    this.centerY = this.width < 900 ? this.height * 0.61 : this.height * 0.47;
    this.systemRadius = Math.min(this.width < 900 ? this.width * 0.42 : this.width * 0.27, this.height * 0.38);
    this.layoutNodes();
    this.buildParticles(this.particleBudget());
    this.draw(performance.now());
  }

  layoutNodes() {
    const mobile = this.width < 900;
    for (let index = 0; index < this.nodes.length; index += 1) {
      const angle = -Math.PI / 2 + (index / this.nodes.length) * Math.PI * 2;
      const radius = this.systemRadius * (index % 2 === 0 ? 0.78 : 1.02);
      this.nodes[index].x = this.centerX + Math.cos(angle) * radius;
      this.nodes[index].y = this.centerY + Math.sin(angle) * radius * (mobile ? 0.73 : 0.82);
      this.nodes[index].radius = mobile ? 18 : 23;
    }
  }

  buildParticles(count) {
    if (this.particles.length === count) return;
    this.particles.length = 0;
    this.random = new DeterministicRandom(0x504f4953);
    for (let index = 0; index < count; index += 1) {
      this.particles.push({
        family: index % this.nodes.length,
        phase: this.random.next() * Math.PI * 2,
        orbit: 16 + this.random.next() * 53,
        speed: 0.00008 + this.random.next() * 0.00017,
        size: 0.45 + this.random.next() * 1.25,
        lane: this.random.next() * 2 - 1,
        drift: this.random.next() * Math.PI * 2,
      });
    }
    document.querySelector("#render-count").textContent = `${count} particles · ${this.dpr.toFixed(2)}× DPR`;
  }

  pointerPosition(event) {
    const rect = this.canvas.getBoundingClientRect();
    return { x: event.clientX - rect.left, y: event.clientY - rect.top };
  }

  hitTest(position) {
    for (const node of this.nodes) {
      const dx = position.x - node.x;
      const dy = position.y - node.y;
      if (dx * dx + dy * dy <= (node.radius + 13) ** 2) return node;
    }
    return null;
  }

  onPointerMove(event) {
    const node = this.hitTest(this.pointerPosition(event));
    const next = node?.id ?? null;
    if (next !== hoveredCapability) {
      hoveredCapability = next;
      this.canvas.style.cursor = node ? "pointer" : "crosshair";
      updateFieldLabel(node?.id ?? selectedCapability);
      if (!playing) this.draw(performance.now());
    }
  }

  onPointerLeave() {
    hoveredCapability = null;
    updateFieldLabel(selectedCapability);
    if (!playing) this.draw(performance.now());
  }

  onClick(event) {
    const node = this.hitTest(this.pointerPosition(event));
    if (node) selectCapability(node.id, true);
  }

  drawGrid(ctx) {
    ctx.save();
    ctx.strokeStyle = "oklch(72% 0.07 120 / 0.10)";
    ctx.lineWidth = 0.7;
    const ringCount = 5;
    for (let ring = 1; ring <= ringCount; ring += 1) {
      ctx.beginPath();
      ctx.ellipse(this.centerX, this.centerY, this.systemRadius * ring / ringCount, this.systemRadius * 0.82 * ring / ringCount, 0, 0, Math.PI * 2);
      ctx.stroke();
    }
    ctx.beginPath();
    ctx.moveTo(this.centerX - this.systemRadius * 1.18, this.centerY);
    ctx.lineTo(this.centerX + this.systemRadius * 1.18, this.centerY);
    ctx.moveTo(this.centerX, this.centerY - this.systemRadius);
    ctx.lineTo(this.centerX, this.centerY + this.systemRadius);
    ctx.stroke();
    ctx.restore();
  }

  drawConnections(ctx, time, event) {
    ctx.save();
    const activeId = hoveredCapability || selectedCapability;
    for (let index = 0; index < this.nodes.length; index += 1) {
      const node = this.nodes[index];
      const next = this.nodes[(index + 1) % this.nodes.length];
      const active = activeId === "all" || node.id === activeId || next.id === activeId;
      ctx.strokeStyle = active ? "oklch(84% 0.175 112 / 0.28)" : "oklch(76% 0.04 120 / 0.08)";
      ctx.lineWidth = active ? 0.9 : 0.6;
      ctx.beginPath();
      if (event.id === "loom") {
        const bend = Math.sin(time * 0.002 + index) * 34;
        ctx.moveTo(node.x, node.y);
        ctx.bezierCurveTo(this.centerX + bend, node.y, this.centerX - bend, next.y, next.x, next.y);
      } else if (event.id === "property") {
        ctx.ellipse(this.centerX, this.centerY, this.systemRadius * (0.42 + index * 0.07), this.systemRadius * (0.28 + index * 0.045), index * 0.17, 0, Math.PI * 2);
      } else if (event.id === "msrv") {
        const y = this.centerY - this.systemRadius * 0.62 + index * this.systemRadius * 0.2;
        ctx.moveTo(this.centerX - this.systemRadius, y);
        ctx.lineTo(this.centerX + this.systemRadius, y);
      } else {
        ctx.moveTo(node.x, node.y);
        ctx.lineTo(this.centerX, this.centerY);
      }
      ctx.stroke();
    }
    ctx.restore();
  }

  drawParticles(ctx, time, event) {
    const activeId = hoveredCapability || selectedCapability;
    ctx.save();
    for (const particle of this.particles) {
      const node = this.nodes[particle.family];
      const familyActive = activeId === "all" || node.id === activeId;
      const motionTime = reduceMotion.matches ? 14000 : time;
      const theta = particle.phase + motionTime * particle.speed * (event.id === "loom" ? 1.7 : 1);
      let x = node.x + Math.cos(theta) * particle.orbit;
      let y = node.y + Math.sin(theta) * particle.orbit * 0.48;

      if (event.id === "ordinary") {
        const travel = (Math.sin(theta + particle.drift) + 1) * 0.5;
        x = node.x + (this.centerX - node.x) * travel * 0.82 + Math.cos(theta * 3) * 5;
        y = node.y + (this.centerY - node.y) * travel * 0.82 + Math.sin(theta * 2) * 5;
      } else if (event.id === "mutation") {
        const split = Math.sin(theta * 3 + particle.drift);
        x += split * particle.orbit * 0.34;
        y += Math.sign(split) * 6;
      } else if (event.id === "loom") {
        const target = this.nodes[(particle.family + 1) % this.nodes.length];
        const travel = (Math.sin(theta) + 1) * 0.5;
        x = node.x + (target.x - node.x) * travel + Math.sin(theta * 4) * 9 * particle.lane;
        y = node.y + (target.y - node.y) * travel + Math.cos(theta * 4) * 9 * particle.lane;
      } else if (event.id === "msrv") {
        const laneY = this.centerY - this.systemRadius * 0.62 + particle.family * this.systemRadius * 0.2;
        x = this.centerX + Math.cos(theta) * this.systemRadius * 0.92;
        y = laneY + Math.sin(theta * 2) * 2;
      }

      const color = event.id === "mutation" && particle.family % 3 === 0
        ? `oklch(69% 0.17 38 / ${familyActive ? 0.76 : 0.12})`
        : `oklch(84% 0.15 ${node.hue} / ${familyActive ? 0.72 : 0.10})`;
      ctx.fillStyle = color;
      ctx.beginPath();
      ctx.arc(x, y, particle.size * (familyActive ? 1 : 0.7), 0, Math.PI * 2);
      ctx.fill();
    }
    ctx.restore();
  }

  drawNodeGlyph(ctx, node, time, active, event) {
    const radius = node.radius + (active ? 4 : 0);
    const pulse = reduceMotion.matches ? 0 : Math.sin(time * 0.002 + node.index) * 2;
    ctx.save();
    ctx.translate(node.x, node.y);
    ctx.strokeStyle = active ? "oklch(84% 0.175 112 / 0.95)" : "oklch(78% 0.06 110 / 0.48)";
    ctx.fillStyle = active ? "oklch(84% 0.175 112 / 0.12)" : "oklch(18% 0.02 145 / 0.72)";
    ctx.lineWidth = active ? 1.5 : 0.8;
    ctx.beginPath();
    ctx.arc(0, 0, radius + pulse, 0, Math.PI * 2);
    ctx.fill();
    ctx.stroke();

    ctx.beginPath();
    if (node.id === "selection") {
      for (let side = 0; side < 6; side += 1) {
        const angle = -Math.PI / 2 + side * Math.PI / 3;
        const x = Math.cos(angle) * radius * 0.58;
        const y = Math.sin(angle) * radius * 0.58;
        if (side === 0) ctx.moveTo(x, y); else ctx.lineTo(x, y);
      }
      ctx.closePath();
    } else if (node.id === "affinity") {
      ctx.moveTo(0, -radius * 0.58); ctx.lineTo(0, radius * 0.5);
      ctx.moveTo(-radius * 0.48, radius * 0.18); ctx.quadraticCurveTo(0, radius * 0.72, radius * 0.48, radius * 0.18);
    } else if (node.id === "topology") {
      ctx.rect(-radius * 0.55, -radius * 0.55, radius * 0.82, radius * 0.82);
      ctx.rect(-radius * 0.27, -radius * 0.27, radius * 0.82, radius * 0.82);
    } else if (node.id === "health") {
      ctx.arc(0, 0, radius * 0.57, Math.PI * 0.15, Math.PI * 0.85);
      ctx.moveTo(-radius * 0.52, radius * 0.18); ctx.arc(0, 0, radius * 0.57, Math.PI * 0.85, Math.PI * 1.15);
      ctx.moveTo(-radius * 0.52, -radius * 0.18); ctx.arc(0, 0, radius * 0.57, Math.PI * 1.15, Math.PI * 1.85);
    } else if (node.id === "discovery") {
      ctx.arc(0, 0, radius * 0.22, 0, Math.PI * 2);
      ctx.moveTo(radius * 0.4, 0); ctx.arc(0, 0, radius * 0.4, 0, Math.PI * 2);
      ctx.moveTo(radius * 0.58, 0); ctx.arc(0, 0, radius * 0.58, 0, Math.PI * 2);
    } else if (node.id === "observe") {
      ctx.ellipse(0, 0, radius * 0.62, radius * 0.34, 0, 0, Math.PI * 2);
      ctx.moveTo(radius * 0.16, 0); ctx.arc(0, 0, radius * 0.16, 0, Math.PI * 2);
    } else {
      ctx.arc(0, radius * 0.14, radius * 0.54, Math.PI, 0);
      ctx.moveTo(-radius * 0.54, radius * 0.14); ctx.lineTo(-radius * 0.54, radius * 0.55);
      ctx.moveTo(radius * 0.54, radius * 0.14); ctx.lineTo(radius * 0.54, radius * 0.55);
    }
    ctx.stroke();

    if (event.id === "mutation") {
      ctx.strokeStyle = "oklch(69% 0.17 38 / 0.95)";
      ctx.beginPath();
      ctx.moveTo(-4, -radius * 0.86); ctx.lineTo(2, -5); ctx.lineTo(-3, 3); ctx.lineTo(5, radius * 0.86);
      ctx.stroke();
    } else if (event.id === "property") {
      ctx.setLineDash([2, 4]);
      ctx.beginPath(); ctx.arc(0, 0, radius * 0.78, 0, Math.PI * 2); ctx.stroke();
    }
    ctx.restore();
  }

  drawCenter(ctx, time, event) {
    const spin = reduceMotion.matches ? 0.32 : time * 0.00008;
    ctx.save();
    ctx.translate(this.centerX, this.centerY);
    ctx.rotate(spin);
    ctx.strokeStyle = "oklch(84% 0.175 112 / 0.9)";
    ctx.fillStyle = "oklch(84% 0.175 112 / 0.12)";
    ctx.lineWidth = 1;
    ctx.beginPath(); ctx.arc(0, 0, 31, 0, Math.PI * 2); ctx.fill(); ctx.stroke();
    ctx.beginPath();
    for (let ray = 0; ray < 8; ray += 1) {
      const angle = ray * Math.PI / 4;
      ctx.moveTo(Math.cos(angle) * 38, Math.sin(angle) * 38);
      ctx.lineTo(Math.cos(angle) * (event.id === "mutation" ? 51 + (ray % 2) * 8 : 48), Math.sin(angle) * (event.id === "mutation" ? 51 + (ray % 2) * 8 : 48));
    }
    ctx.stroke();
    ctx.rotate(-spin);
    ctx.fillStyle = "oklch(93% 0.018 88)";
    ctx.font = `500 ${this.width < 580 ? 8 : 9}px Geologica, sans-serif`;
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillText("POISE", 0, 1);
    ctx.restore();
  }

  drawLabels(ctx, activeId) {
    if (this.width < 580) return;
    ctx.save();
    ctx.font = "500 9px 'Atkinson Hyperlegible Next', sans-serif";
    ctx.letterSpacing = "0.08em";
    ctx.textAlign = "center";
    for (const node of this.nodes) {
      const active = activeId === "all" || node.id === activeId;
      ctx.fillStyle = active ? "oklch(93% 0.018 88 / 0.92)" : "oklch(79% 0.023 88 / 0.45)";
      ctx.fillText(node.name.toUpperCase(), node.x, node.y + node.radius + 18);
    }
    ctx.restore();
  }

  draw(time) {
    const ctx = this.ctx;
    const event = EVENTS[eventIndex];
    const activeId = hoveredCapability || selectedCapability;
    ctx.clearRect(0, 0, this.width, this.height);
    this.drawGrid(ctx);
    this.drawConnections(ctx, time, event);
    this.drawParticles(ctx, time, event);
    for (const node of this.nodes) {
      this.drawNodeGlyph(ctx, node, time, activeId === "all" || activeId === node.id, event);
    }
    this.drawCenter(ctx, time, event);
    this.drawLabels(ctx, activeId);
  }
}

const orreryRenderer = context ? new Orrery(canvas, context) : null;

function resetTimelineAnimation() {
  const active = eventButtons[eventIndex];
  active.classList.remove("is-active");
  void active.offsetWidth;
  active.classList.add("is-active");
}

function updateEvent(announce = true) {
  const event = EVENTS[eventIndex];
  document.querySelector("#event-index").textContent = `${String(eventIndex + 1).padStart(2, "0")} / ${String(EVENTS.length).padStart(2, "0")}`;
  document.querySelector("#event-title").textContent = event.title;
  document.querySelector("#event-detail").textContent = evidenceFor(event);
  eventButtons.forEach((button, index) => {
    button.classList.toggle("is-active", index === eventIndex);
    button.classList.toggle("is-complete", index < eventIndex);
    button.setAttribute("aria-pressed", String(index === eventIndex));
  });
  document.querySelectorAll("[data-proof]").forEach((article) => {
    article.classList.toggle("is-current", article.dataset.proof === event.id);
  });
  eventStartedAt = performance.now();
  resetTimelineAnimation();
  if (announce) liveDescription.textContent = `${event.title}: ${evidenceFor(event)}.`;
  if (!playing) orreryRenderer?.draw(performance.now());
}

function setEvent(nextIndex, announce = true) {
  eventIndex = (nextIndex + EVENTS.length) % EVENTS.length;
  updateEvent(announce);
}

function setPlaying(nextPlaying) {
  playing = nextPlaying && !reduceMotion.matches;
  orrery.classList.toggle("is-paused", !playing);
  replayButton.querySelector(".play-symbol").textContent = playing ? "Ⅱ" : "▶";
  replayButton.querySelector(".play-label").textContent = playing ? "Pause replay" : "Resume replay";
  replayButton.setAttribute("aria-pressed", String(!playing));
  eventStartedAt = performance.now();
  if (playing) {
    resetTimelineAnimation();
    ensureFrame();
  } else {
    cancelAnimationFrame(frameRequest);
    frameRequest = 0;
    orreryRenderer?.draw(performance.now());
  }
}

function updateFieldLabel(id) {
  const capability = CAPABILITIES.find((item) => item.id === id);
  const label = document.querySelector("#field-label strong");
  label.textContent = capability?.name ?? "All capabilities";
}

function selectCapability(id, announce = false) {
  selectedCapability = id;
  capabilityButtons.forEach((button) => {
    const selected = button.dataset.capability === id;
    button.classList.toggle("is-active", selected);
    button.setAttribute("aria-pressed", String(selected));
  });
  updateFieldLabel(id);
  if (announce) {
    const capability = CAPABILITIES.find((item) => item.id === id);
    liveDescription.textContent = capability ? `${capability.name}: ${capability.detail}. The orrery has reorganized around this field.` : "All capability fields are visible.";
  }
  if (!playing) orreryRenderer?.draw(performance.now());
}

function ensureFrame() {
  if (!frameRequest && playing && inView && !document.hidden && orreryRenderer) {
    frameRequest = requestAnimationFrame(frame);
  }
}

function frame(time) {
  frameRequest = 0;
  if (!playing || !inView || document.hidden || !orreryRenderer) return;
  if (time - lastFrame >= 1000 / 60) {
    orreryRenderer.draw(time);
    lastFrame = time;
  }
  if (time - eventStartedAt >= 5800) setEvent(eventIndex + 1, false);
  ensureFrame();
}

eventButtons.forEach((button) => {
  button.addEventListener("click", () => setEvent(Number(button.dataset.event)));
});

capabilityButtons.forEach((button, index) => {
  button.setAttribute("aria-keyshortcuts", index === 0 ? "0" : String(index));
  button.addEventListener("click", () => selectCapability(button.dataset.capability, true));
  button.addEventListener("pointerenter", () => {
    hoveredCapability = button.dataset.capability;
    updateFieldLabel(hoveredCapability);
  });
  button.addEventListener("pointerleave", () => {
    hoveredCapability = null;
    updateFieldLabel(selectedCapability);
  });
});

replayButton.addEventListener("click", () => setPlaying(!playing));

document.addEventListener("keydown", (event) => {
  const target = event.target;
  if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement) return;
  if (event.key === " " && !target.closest("a, button")) {
    event.preventDefault();
    setPlaying(!playing);
  } else if (event.key === "ArrowRight") {
    setEvent(eventIndex + 1);
  } else if (event.key === "ArrowLeft") {
    setEvent(eventIndex - 1);
  } else if (/^[0-7]$/.test(event.key)) {
    selectCapability(capabilityButtons[Number(event.key)]?.dataset.capability ?? "all", true);
  }
});

document.addEventListener("visibilitychange", ensureFrame);
new IntersectionObserver(([entry]) => {
  inView = entry.isIntersecting;
  if (inView) ensureFrame();
}, { threshold: 0.02 }).observe(orrery);

reduceMotion.addEventListener("change", () => {
  orreryRenderer?.resize();
  setPlaying(!reduceMotion.matches);
});

selectCapability("all");
updateEvent(false);
setPlaying(playing);
loadRecord().finally(() => {
  document.documentElement.dataset.state = context ? "ready" : "fallback";
  if (!context) document.querySelector("#run-state").textContent += " · static renderer";
  ensureFrame();
});
