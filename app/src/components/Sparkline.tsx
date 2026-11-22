// Activity density as a small filled curve (library cards, details).
import { useMemo } from "react";

export function Sparkline({
  data,
  color = "var(--gold-400)",
  height = 34,
  className = "",
}: {
  data: number[];
  color?: string;
  height?: number;
  className?: string;
}) {
  const { line, area } = useMemo(() => {
    const n = data.length;
    if (!n) return { line: "", area: "" };
    const max = Math.max(1, ...data);
    const w = 100;
    const pts = data.map((v, i) => {
      const x = n === 1 ? w / 2 : (i / (n - 1)) * w;
      // sqrt keeps quiet parts visible next to bursts
      const y = height - 2 - Math.sqrt(v / max) * (height - 5);
      return [x, y] as const;
    });
    let d = `M${pts[0][0]},${pts[0][1]}`;
    for (let i = 1; i < pts.length; i++) {
      const [x0, y0] = pts[i - 1];
      const [x1, y1] = pts[i];
      const cx = (x0 + x1) / 2;
      d += ` C${cx},${y0} ${cx},${y1} ${x1},${y1}`;
    }
    return { line: d, area: `${d} L${pts[pts.length - 1][0]},${height} L${pts[0][0]},${height} Z` };
  }, [data, height]);
  const gid = useMemo(() => `sg${Math.random().toString(36).slice(2, 9)}`, []);
  if (!data.length) return <div className={`sparkline empty ${className}`} style={{ height }} />;
  return (
    <svg className={`sparkline ${className}`} viewBox={`0 0 100 ${height}`} preserveAspectRatio="none" style={{ height }}>
      <defs>
        <linearGradient id={gid} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor={color} stopOpacity="0.42" />
          <stop offset="100%" stopColor={color} stopOpacity="0" />
        </linearGradient>
      </defs>
      <path d={area} fill={`url(#${gid})`} />
      <path d={line} fill="none" stroke={color} strokeWidth="1.4" vectorEffect="non-scaling-stroke" />
    </svg>
  );
}
