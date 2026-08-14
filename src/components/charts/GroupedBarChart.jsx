import React, { useMemo } from 'react';
import { useT } from '../../hooks/useT';

const BAR_W = 28;
const GROUP_GAP = 80;
const PAD_X = 32;
const PAD_TOP = 16;
const AXIS_H = 32;
const Y_TICKS = 4;
const MIN_WIDTH = 720;

export default function GroupedBarChart({ tasks, history }) {
  const t = useT();
  const data = useMemo(() => {
    return (tasks || []).map((t) => {
      const runs = history.filter((h) => h.taskId === t.id);
      const success = runs.filter((h) => h.status === 'success').length;
      const error = runs.filter((h) => h.status === 'error').length;
      return { id: t.id, name: t.name, success, error };
    });
  }, [tasks, history]);

  const maxCount = useMemo(
    () => Math.max(1, ...data.map((d) => Math.max(d.success, d.error))),
    [data],
  );

  const chartHeight = 200;
  const plotH = chartHeight - PAD_TOP - AXIS_H;
  const groupW = BAR_W * 2 + 6;
  const innerW = data.length * (groupW + GROUP_GAP);
  const width = Math.max(innerW + PAD_X * 2, MIN_WIDTH);

  if (data.length === 0) {
    return <div className="chart-empty">{t('chart.empty.tasks')}</div>;
  }

  const yTicksRaw = Array.from({ length: Y_TICKS + 1 }, (_, i) => Math.round((maxCount * i) / Y_TICKS));
  const yTicks = Array.from(new Set(yTicksRaw)).sort((a, b) => a - b);

  return (
    <div className="grouped-bars">
      <div className="grouped-bars__yaxis" aria-hidden="true">
        {yTicks.slice().reverse().map((v, i) => (
          <div key={i} className="grouped-bars__ytick">{v}</div>
        ))}
      </div>
      <div className="grouped-bars__scroll">
        <svg width={width} height={chartHeight} role="img" aria-label={t('statistics.aria.bars')}>
          {yTicks.map((v, i) => {
            const y = maxCount > 0 ? PAD_TOP + plotH - (v / maxCount) * plotH : PAD_TOP + plotH;
            return (
              <line
                key={i}
                x1={PAD_X}
                x2={width - PAD_X}
                y1={y}
                y2={y}
                stroke="var(--separator)"
                strokeWidth="0.5"
              />
            );
          })}
          {data.map((d, i) => {
            const baseX = PAD_X + i * (groupW + GROUP_GAP);
            const sH = (d.success / maxCount) * plotH;
            const eH = (d.error / maxCount) * plotH;
            const sY = PAD_TOP + plotH - sH;
            const eY = PAD_TOP + plotH - eH;
            return (
              <g key={d.id}>
                <rect x={baseX} y={sY} width={BAR_W} height={Math.max(sH, 2)} rx="2" fill="var(--accent)" opacity="0.85">
                  <title>{`${d.name} — ${d.success} ${t('chart.legend.success').toLowerCase()}`}</title>
                </rect>
                <rect x={baseX + BAR_W + 4} y={eY} width={BAR_W} height={Math.max(eH, 2)} rx="2" fill="var(--system-red)" opacity="0.85">
                  <title>{`${d.name} — ${d.error} ${t('chart.legend.error').toLowerCase()}`}</title>
                </rect>
                <text
                  x={baseX + groupW / 2}
                  y={chartHeight - 14}
                  textAnchor="middle"
                  fontSize="11"
                  fill="var(--label-secondary)"
                >
                  {d.name}
                </text>
              </g>
            );
          })}
        </svg>
      </div>
      <div className="grouped-bars__legend">
        <span><span className="legend-dot" style={{ background: 'var(--accent)' }} /> {t('chart.legend.success')}</span>
        <span><span className="legend-dot" style={{ background: 'var(--system-red)' }} /> {t('chart.legend.error')}</span>
      </div>
    </div>
  );
}
