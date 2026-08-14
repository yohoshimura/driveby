import React from 'react';

export default function Toggle({ value, onChange, disabled, label }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={value}
      aria-label={label}
      onClick={() => !disabled && onChange(!value)}
      disabled={disabled}
      className={`toggle ${value ? 'toggle--on' : ''} ${disabled ? 'toggle--disabled' : ''}`}
    >
      <span className="toggle__knob" />
    </button>
  );
}
