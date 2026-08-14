import React, { useMemo, useRef, useEffect } from 'react';
import { formatBytes } from '../../lib/format';
import { useT } from '../../hooks/useT';

const PIE_SIZE = 140;
const PALETTE = [
  '#007aff', '#5856d6', '#af52de', '#ff2d55', '#ff3b30',
  '#ff9500', '#ffcc00', '#34c759', '#00c7be', '#30b0c7',
  '#a2845e', '#8e8e93',
];

function colorForTask(taskId, lookup) {
  if (lookup.has(taskId)) return lookup.get(taskId);
  const c = PALETTE[lookup.size % PALETTE.length];
  lookup.set(taskId, c);
  return c;
}

function arcPath(cx, cy, r, startAngle, endAngle) {
  const polar = (a) => [cx + r * Math.cos(a - Math.PI / 2), cy + r * Math.sin(a - Math.PI / 2)];
  const [x1, y1] = polar(startAngle);
  const [x2, y2] = polar(endAngle);
  const large = endAngle - startAngle > Math.PI ? 1 : 0;
  return `M ${cx} ${cy} L ${x1} ${y1} A ${r} ${r} 0 ${large} 1 ${x2} ${y2} Z`;
}

export default function CandleChart({ entries }) {
  const t = useT();
  const { days, taskColors, taskNames } = useMemo(() => {
    const byDay = new Map(); // dayKey -> Map<taskId, { name, bytes }>
    const names = new Map();
    for (const e of entries || []) {
      if (!e.timestamp) continue;
      const d = new Date(e.timestamp);
      const dayKey = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
      if (!byDay.has(dayKey)) byDay.set(dayKey, new Map());
      const dayMap = byDay.get(dayKey);
      const id = e.taskId || 'unknown';
      const prev = dayMap.get(id) || { name: e.taskName || 'Task', bytes: 0 };
      prev.bytes += e.totalBytes || 0;
      prev.name = e.taskName || prev.name;
      dayMap.set(id, prev);
      names.set(id, e.taskName || 'Task');
    }
    const sorted = Array.from(byDay.entries())
      .sort((a, b) => a[0].localeCompare(b[0]))
      .map(([dayKey, taskMap]) => {
        const slices = Array.from(taskMap.entries()).map(([id, v]) => ({
          taskId: id,
          name: v.name,
          bytes: v.bytes,
        }));
        const total = slices.reduce((s, x) => s + x.bytes, 0);
        return { dayKey, slices, total };
      });
    const colors = new Map();
    for (const day of sorted) {
      for (const s of day.slices) colorForTask(s.taskId, colors);
    }
    return { days: sorted, taskColors: colors, taskNames: names };
  }, [entries]);

  const scrollRef = useRef(null);
  useEffect(() => {
    if (scrollRef.current) scrollRef.current.scrollLeft = scrollRef.current.scrollWidth;
  }, [days.length]);

  if (days.length === 0) {
    return <div className="chart-empty">{t('chart.empty.backups')}</div>;
  }

  const r = (PIE_SIZE - 16) / 2;
  const cx = PIE_SIZE / 2;
  const cy = PIE_SIZE / 2;

  return (
    <div className="day-pies">
      <div className="day-pies__scroll" ref={scrollRef}>
        <div className="day-pies__row">
          {days.map((day) => {
            let acc = 0;
            return (
              <div className="day-pie" key={day.dayKey}>
                <svg
                  width={PIE_SIZE}
                  height={PIE_SIZE}
                  role="img"
                  aria-label={t('statistics.aria.day', { day: day.dayKey, bytes: formatBytes(day.total) })}
                >
                  {day.total > 0 ? (
                    // Single-slice day: SVG arcs degenerate when start and
                    // end map to the same point, so render a full circle
                    // instead — otherwise the pie just doesn't draw (#9).
                    day.slices.length === 1 ? (
                      <circle
                        key={day.slices[0].taskId}
                        cx={cx}
                        cy={cy}
                        r={r}
                        fill={taskColors.get(day.slices[0].taskId)}
                        opacity="0.92"
                        stroke="var(--bg-content)"
                        strokeWidth="1"
                      >
                        <title>{`${day.slices[0].name} — ${formatBytes(day.slices[0].bytes)}`}</title>
                      </circle>
                    ) : day.slices.map((s) => {
                      const start = (acc / day.total) * Math.PI * 2;
                      acc += s.bytes;
                      const end = (acc / day.total) * Math.PI * 2;
                      const color = taskColors.get(s.taskId);
                      return (
                        <path
                          key={s.taskId}
                          d={arcPath(cx, cy, r, start, end)}
                          fill={color}
                          opacity="0.92"
                          stroke="var(--bg-content)"
                          strokeWidth="1"
                        >
                          <title>{`${s.name} — ${formatBytes(s.bytes)}`}</title>
                        </path>
                      );
                    })
                  ) : (
                    <circle cx={cx} cy={cy} r={r} fill="var(--separator)" opacity="0.3" />
                  )}
                </svg>
                <div className="day-pie__total">{formatBytes(day.total)}</div>
                <div className="day-pie__date">
                  {day.dayKey.slice(8, 10)}/{day.dayKey.slice(5, 7)}
                </div>
              </div>
            );
          })}
        </div>
      </div>
      {taskColors.size > 0 && (
        <div className="day-pies__legend">
          {Array.from(taskColors.entries()).map(([id, color]) => (
            <span key={id}>
              <span className="legend-dot" style={{ background: color }} />
              {taskNames.get(id) || 'Task'}
            </span>
          ))}
        </div>
      )}
    </div>
  );
}
