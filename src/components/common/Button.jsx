import React from 'react';

export default function Button({
  children,
  onClick,
  variant = 'default',
  size,
  disabled,
  type = 'button',
  ariaLabel,
  destructive,
}) {
  const classes = [
    'btn',
    variant === 'primary' ? 'btn--primary' : '',
    variant === 'borderless' ? 'btn--borderless' : '',
    size === 'large' ? 'btn--large' : '',
    size === 'small' ? 'btn--small' : '',
    destructive ? 'btn--destructive' : '',
  ].filter(Boolean).join(' ');

  return (
    <button
      type={type}
      onClick={onClick}
      disabled={disabled}
      aria-label={ariaLabel}
      className={classes}
    >
      {children}
    </button>
  );
}
