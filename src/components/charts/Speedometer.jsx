import React from 'react';

function pt(cx, cy, r, deg) {
  const rad = ((deg - 180) * Math.PI) / 180;
  return { x: cx + r * Math.cos(rad), y: cy + r * Math.sin(rad) };
}

function arc(cx, cy, r, from, to, color, width) {
  const s = pt(cx, cy, r, from);
  const e = pt(cx, cy, r, to);
  const large = to - from > 180 ? 1 : 0;
  return (
    <path
      d={`M ${s.x} ${s.y} A ${r} ${r} 0 ${large} 1 ${e.x} ${e.y}`}
      fill="none"
      stroke={color}
      strokeWidth={width}
      strokeLinecap="round"
    />
  );
}

export default function Speedometer({ successes = 0, failures = 0, size = 96 }) {
  const cx = size / 2;
  const cy = size * 0.62;
  const r = size / 2 - 6;

  const failOpacity = failures > 0 ? 0.55 : 0.18;
  const successOpacity = successes > 0 ? 0.55 : 0.18;

  const needle = pt(cx, cy, r - 4, 90);

  return (
    <div className="chart">
      <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} role="img">
        <g style={{ opacity: failOpacity }}>
          {arc(cx, cy, r, 0, 90, 'var(--system-red)', 6)}
        </g>
        <g style={{ opacity: successOpacity }}>
          {arc(cx, cy, r, 90, 180, 'var(--system-green)', 6)}
        </g>
        <line
          x1={cx} y1={cy}
          x2={needle.x} y2={needle.y}
          stroke="var(--label-primary)"
          strokeWidth="1.5"
          strokeLinecap="round"
        />
        <circle cx={cx} cy={cy} r="3" fill="var(--label-primary)" />
      </svg>
    </div>
  );
}
