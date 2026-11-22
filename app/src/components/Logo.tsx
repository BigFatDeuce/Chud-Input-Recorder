// The brand logo, always using the file meant for the displayed size
// (logo.svg ≥ 128 px, logo-small.svg 32–64 px, logo-tiny.svg 16–24 px).

export function Logo({ size, className }: { size: number; className?: string }) {
  const src = size >= 128 ? "/brand/logo.svg" : size > 24 ? "/brand/logo-small.svg" : "/brand/logo-tiny.svg";
  return <img src={src} width={size} height={size} alt="Chud Input Recorder" className={className} draggable={false} />;
}
