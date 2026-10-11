// The editor's preview. Rust computes every frame (render::Scene::track) and this script only
// looks them up as the video plays: no layout math lives here, so the preview shows what the
// export will render.
//
// Elements (from editor.rs): #sv-stage, #sv-content, #sv-video, #sv-cursor, #sv-timeline,
// #sv-playhead, #sv-time. Rust never sets the style properties this script owns.
//
// Messages to Rust, through `sv.send`: {kind: "tick", t, playing} and {kind: "key", key, cmd, shift}.
(() => {
  if (window.sv) return;
  const STRIDE = 6;
  const pct = (v, of) => `${(v / of) * 100}%`;
  const clock = (s) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, "0")}`;

  const sv = (window.sv = {
    track: null,
    send: null,
    sent: { t: -1, playing: false },
    scrubbing: false,

    setTrack(track) {
      this.track = track;
    },
    video() {
      return document.getElementById("sv-video");
    },
    /// Length of the recording (the timeline shows all of it, cuts included).
    duration() {
      const t = this.track;
      return t ? (t.frames.length / STRIDE - 1) / t.fps : 0;
    },
    toggle() {
      const v = this.video();
      if (v) v.paused ? v.play().catch(() => {}) : v.pause();
    },
    seek(t) {
      const v = this.video();
      if (v) v.currentTime = Math.max(0, Math.min(t, this.duration()));
    },

    // The parts that only change with the style; cheap enough to apply every frame, which also
    // covers elements Dioxus has just re-created.
    layout(t) {
      const stage = document.getElementById("sv-stage");
      const content = document.getElementById("sv-content");
      if (!stage || !content) return;
      stage.style.setProperty("--ratio", t.width / t.height);
      stage.style.aspectRatio = `${t.width} / ${t.height}`;
      const c = t.content;
      content.style.left = pct(c.x, t.width);
      content.style.top = pct(c.y, t.height);
      content.style.width = pct(c.w, t.width);
      content.style.height = pct(c.h, t.height);
      content.style.borderRadius = `${pct(t.radius, c.w)} / ${pct(t.radius, c.h)}`;
      // In output pixels, scaled to the stage. A CSS blur radius is twice the Gaussian's sigma.
      const k = stage.clientWidth / t.width;
      const s = t.shadow;
      content.style.boxShadow = s ? `0 ${s.dy * k}px ${2 * s.sigma * k}px rgba(0, 0, 0, ${s.alpha})` : "none";
    },

    draw() {
      // One bad frame must not stop the loop.
      try {
        this.drawFrame();
      } catch (e) {
        console.error(e);
      }
      requestAnimationFrame(() => this.draw());
    },

    drawFrame() {
      const t = this.track;
      const v = this.video();
      if (t && v) {
        this.layout(t);
        // WebKit doesn't paint a paused video until it's been positioned once.
        if (v.readyState >= 1 && !v.dataset.primed) {
          v.dataset.primed = "1";
          v.currentTime = Math.max(v.currentTime, 0.001);
        }
        const last = t.frames.length / STRIDE - 1;
        // The video's speed: the browser plays faster at the same pitch, as the export does.
        const speed = t.speed || 1;
        if (v.playbackRate !== speed) {
          v.playbackRate = speed;
          v.preservesPitch = true;
        }
        let time = v.currentTime;
        // Jump over the blanks cut while playing; paused, a cut can still be looked at.
        const skip = (t.skips || []).find(([s, e]) => time >= s && time < e - 0.001);
        if (skip && !v.paused) {
          if (skip[1] >= this.duration() - 0.01) {
            v.pause();
          } else {
            v.currentTime = time = skip[1];
          }
        }
        const f = Math.max(0, time * t.fps);
        const i = Math.min(Math.floor(f), last);
        const j = Math.min(i + 1, last);
        const k = Math.min(f - i, 1);
        const at = (n) => t.frames[i * STRIDE + n] + (t.frames[j * STRIDE + n] - t.frames[i * STRIDE + n]) * k;

        const size = at(2);
        v.style.transform = `scale(${1 / size}) translate(${-at(0) * 100}%, ${-at(1) * 100}%)`;

        const cursor = document.getElementById("sv-cursor");
        if (cursor) {
          // Not interpolated into or out of view: it would slide in from the corner.
          const visible = t.frames[i * STRIDE + 5] > 0 && t.frames[j * STRIDE + 5] > 0;
          cursor.style.display = visible ? "block" : "none";
          if (visible) {
            cursor.style.left = pct(at(3), t.width);
            cursor.style.top = pct(at(4), t.height);
            cursor.style.height = pct(at(5), t.height);
          }
        }

        const playhead = document.getElementById("sv-playhead");
        if (playhead) playhead.style.left = pct(Math.min(time, this.duration()), this.duration() || 1);
        const label = document.getElementById("sv-time");
        // Video time, as exported: the recording time less what's cut before it, at the speed.
        const cutBefore = (t.skips || []).reduce((sum, [s, e]) => sum + Math.max(0, Math.min(e, time) - s), 0);
        if (label) label.textContent = clock((time - cutBefore) / speed);

        const playing = !v.paused;
        if (this.send && (playing !== this.sent.playing || Math.abs(time - this.sent.t) > 0.1)) {
          this.sent = { t: time, playing };
          this.send({ kind: "tick", t: time, playing });
        }
      }
    },

    scrub(e) {
      const timeline = document.getElementById("sv-timeline");
      if (!timeline) return;
      const r = timeline.getBoundingClientRect();
      this.seek(((e.clientX - r.left) / r.width) * this.duration());
    },
  });

  // Click or drag on the timeline (outside a zoom) to seek.
  document.addEventListener("pointerdown", (e) => {
    if (!e.target.closest("#sv-timeline") || e.target.closest(".zoom, .cut")) return;
    sv.scrubbing = true;
    sv.scrub(e);
  });
  document.addEventListener("pointermove", (e) => sv.scrubbing && sv.scrub(e));
  document.addEventListener("pointerup", () => (sv.scrubbing = false));

  // Keys, unless a text field or slider has them. Space is handled here so a focused button
  // doesn't also get clicked.
  document.addEventListener("keydown", (e) => {
    if (e.target.closest("input, textarea, select")) return;
    if (e.code === "Space") {
      e.preventDefault();
      sv.toggle();
      return;
    }
    if (e.code === "ArrowLeft" || e.code === "ArrowRight") {
      e.preventDefault();
      const v = sv.video();
      if (v) sv.seek(v.currentTime + (e.code === "ArrowLeft" ? -1 : 1) * (e.shiftKey ? 5 : 1));
      return;
    }
    if (sv.send) sv.send({ kind: "key", key: e.key, cmd: e.metaKey || e.ctrlKey, shift: e.shiftKey });
  });

  sv.draw();
})();
