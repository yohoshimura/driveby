import React from 'react';

export default function InfoTip({ text, placement = 'left' }) {
  if (!text) return null;
  return (
    <span
      className={`info-tip info-tip--${placement}`}
      data-tip={text}
      aria-label={text}
      role="img"
      tabIndex={0}
    >
      i
    </span>
  );
}
