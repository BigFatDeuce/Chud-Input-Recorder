import { useEffect, useRef } from "react";

interface Star {
  x: number;
  y: number;
  z: number;
  tw: number;
}

/** Slow parallax starfield with a faint nebula, drawn behind everything. */
export function Starfield({ enabled, still }: { enabled: boolean; still: boolean }) {
  const ref = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = ref.current;
    if (!canvas || !enabled) return;
    const ctx = canvas.getContext("2d")!;
    let raf = 0;
    let w = 0;
    let h = 0;
    let stars: Star[] = [];
    const dpr = Math.min(window.devicePixelRatio || 1, 2);

    const resize = () => {
      w = canvas.clientWidth;
      h = canvas.clientHeight;
      canvas.width = w * dpr;
      canvas.height = h * dpr;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      const n = Math.round((w * h) / 5200);
      stars = Array.from({ length: n }, () => ({
        x: Math.random() * w,
        y: Math.random() * h,
        z: Math.random() ** 2.2,
        tw: Math.random() * Math.PI * 2,
      }));
    };
    resize();
    const ro = new ResizeObserver(resize);
    ro.observe(canvas);

    let last = performance.now();
    const draw = (now: number) => {
      const dt = Math.min(64, now - last);
      last = now;
      ctx.clearRect(0, 0, w, h);
      for (const s of stars) {
        if (!still) {
          s.x -= (0.004 + s.z * 0.018) * dt;
          if (s.x < -2) {
            s.x = w + 2;
            s.y = Math.random() * h;
          }
          s.tw += dt * 0.0016;
        }
        const a = (0.18 + s.z * 0.75) * (0.72 + 0.28 * Math.sin(s.tw));
        const r = 0.35 + s.z * 1.15;
        ctx.globalAlpha = a;
        ctx.fillStyle = s.z > 0.82 ? "#ffe486" : s.z > 0.6 ? "#bfefff" : "#e8eef6";
        ctx.beginPath();
        ctx.arc(s.x, s.y, r, 0, Math.PI * 2);
        ctx.fill();
      }
      ctx.globalAlpha = 1;
      if (!still) raf = requestAnimationFrame(draw);
    };
    raf = requestAnimationFrame(draw);

    // Pause while hidden: no work in the background.
    const vis = () => {
      cancelAnimationFrame(raf);
      if (!document.hidden) {
        last = performance.now();
        raf = requestAnimationFrame(draw);
      }
    };
    document.addEventListener("visibilitychange", vis);
    return () => {
      cancelAnimationFrame(raf);
      ro.disconnect();
      document.removeEventListener("visibilitychange", vis);
    };
  }, [enabled, still]);

  return (
    <div className="cosmos" aria-hidden>
      <div className="nebula n1" />
      <div className="nebula n2" />
      <div className="nebula n3" />
      {enabled && <canvas ref={ref} className="stars" />}
      <div className="vignette" />
    </div>
  );
}
