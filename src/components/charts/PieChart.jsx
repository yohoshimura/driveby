import React from 'react';

function arc(cx, cy, r, startAngle, endAngle) {
  const start = polar(cx, cy, r, endAngle);
  const end = polar(cx, cy, r, startAngle);
  const largeArc = endAngle - startAngle <= 180 ? 0 : 1;
  return `M ${start.x} ${start.y} A ${r} ${r} 0 ${largeArc} 0 ${end.x} ${end.y} L ${cx} ${cy} Z`;
}
function polar(cx, cy, r, deg) {
  const rad = ((deg - 90) * Math.PI) / 180;
  return { x: cx + r * Math.cos(rad), y: cy + r * Math.sin(rad) };
}

export default function PieChart({ slices, centerLabel, centerValue, size = 96 }) {
  const total = slices.reduce((s, x) => s + Math.max(0, x.value), 0);
  const cx = size / 2, cy = size / 2;
  const r = size / 2 - 2;
  const ri = r * 0.62;

  let acc = 0;
  const paths = total > 0
    ? slices.map((s, i) => {
        const start = (acc / total) * 360;
        acc += Math.max(0, s.value);
        const end = (acc / total) * 360;
        return <path key={i} d={arc(cx, cy, r, start, end === 360 ? 359.999 : end)} fill={s.color} />;
      })
    : [<circle key="empty" cx={cx} cy={cy} r={r} fill="var(--label-quaternary)" />];

  return (
    <div className="chart">
      <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} role="img">
        {paths}
        <circle cx={cx} cy={cy} r={ri} fill="var(--bg-content)" />
      </svg>
      <div className="chart__center">
        <div className="chart__value">{centerValue}</div>
        {centerLabel && <div className="chart__label">{centerLabel}</div>}
      </div>
    </div>
  );
}
