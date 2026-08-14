import React from 'react';

export default function ProcessCycle({ count, centerLabel, size = 96 }) {
  const cx = size / 2, cy = size / 2;
  const r = size / 2 - 8;
  const nodes = Math.min(Math.max(count, 1), 8);

  const dots = Array.from({ length: nodes }, (_, i) => {
    const ang = (i / nodes) * 2 * Math.PI - Math.PI / 2;
    return {
      x: cx + r * Math.cos(ang),
      y: cy + r * Math.sin(ang),
    };
  });

  const start = 8;
  const end = 360 - 8;
  const startRad = ((start - 90) * Math.PI) / 180;
  const endRad = ((end - 90) * Math.PI) / 180;
  const ringPath = `M ${cx + r * Math.cos(startRad)} ${cy + r * Math.sin(startRad)}
                    A ${r} ${r} 0 1 1 ${cx + r * Math.cos(endRad)} ${cy + r * Math.sin(endRad)}`;
  const arrowAng = ((end - 90) * Math.PI) / 180;
  const ax = cx + r * Math.cos(arrowAng);
  const ay = cy + r * Math.sin(arrowAng);
  const tx = Math.cos(arrowAng + Math.PI / 2);
  const ty = Math.sin(arrowAng + Math.PI / 2);

  return (
    <div className="chart">
      <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} role="img">
        <path d={ringPath} fill="none" stroke="var(--accent)" strokeWidth="1.5" strokeLinecap="round" opacity="0.5" />
        <polygon
          points={`${ax},${ay} ${ax - 5 * tx + 3 * Math.cos(arrowAng)},${ay - 5 * ty + 3 * Math.sin(arrowAng)} ${ax + 5 * tx + 3 * Math.cos(arrowAng)},${ay + 5 * ty + 3 * Math.sin(arrowAng)}`}
          fill="var(--accent)"
        />
        {dots.map((d, i) => (
          <circle key={i} cx={d.x} cy={d.y} r={count > 0 ? 4 : 3} fill={count > 0 ? 'var(--accent)' : 'var(--label-quaternary)'} />
        ))}
      </svg>
      <div className="chart__center">
        <div className="chart__value">{count}</div>
        <div className="chart__label">{centerLabel}</div>
      </div>
    </div>
  );
}
