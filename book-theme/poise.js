(() => {
  "use strict";

  const ready = (callback) => {
    if (document.readyState === "loading") {
      document.addEventListener("DOMContentLoaded", callback, { once: true });
    } else {
      callback();
    }
  };

  ready(() => {
    const title = document.querySelector(".menu-title");
    if (title && !title.querySelector(".poise-mark")) {
      const mark = document.createElement("span");
      mark.className = "poise-mark";
      mark.setAttribute("aria-hidden", "true");
      title.prepend(mark);
    }

    const rightButtons = document.querySelector(".right-buttons");
    if (rightButtons) {
      const orrery = document.createElement("a");
      orrery.className = "poise-orrery-link";
      orrery.href = new URL("../", window.location.href).href;
      orrery.textContent = "Invariant Orrery ↗";
      orrery.setAttribute("aria-label", "Open the Poise Invariant Orrery");
      rightButtons.prepend(orrery);
    }

    document.querySelectorAll(".content main h2").forEach((heading, index) => {
      heading.dataset.poiseSection = String(index + 1).padStart(2, "0");
    });

    document.querySelectorAll(".content main a[href]").forEach((link) => {
      const url = new URL(link.href, window.location.href);
      if (url.origin !== window.location.origin) {
        link.rel = "noopener noreferrer";
      }
    });

    const progress = document.createElement("div");
    progress.className = "poise-reading-progress";
    progress.setAttribute("aria-hidden", "true");
    document.body.append(progress);

    let scheduled = false;
    const updateProgress = () => {
      scheduled = false;
      const root = document.documentElement;
      const range = root.scrollHeight - root.clientHeight;
      const ratio = range > 0 ? Math.min(1, Math.max(0, root.scrollTop / range)) : 0;
      progress.style.transform = `scaleX(${ratio})`;
    };
    const scheduleProgress = () => {
      if (!scheduled) {
        scheduled = true;
        window.requestAnimationFrame(updateProgress);
      }
    };

    window.addEventListener("scroll", scheduleProgress, { passive: true });
    window.addEventListener("resize", scheduleProgress, { passive: true });
    updateProgress();
  });
})();
