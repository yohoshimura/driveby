import React from 'react';
import { useExitTransition } from '../hooks/useExitTransition';
import { useT } from '../hooks/useT';

// How long the intro takes to play out, in ms. App.jsx holds the splash open
// for at least this long: loading three local JSON files finishes in a few
// milliseconds, so gating purely on `loaded` would cut the animation off
// before it had started. Keep in sync with the animation delays in
// styles.css (last LED starts at 0.90s and runs 0.28s).
export const SPLASH_MS = 1200;

/**
 * Launch animation: the drive scales in, the arrow drops into it, then the
 * three status LEDs light up in sequence.
 *
 * The SVG geometry below is a hand-kept copy of src-tauri/icons/logo.svg.
 * It is inlined rather than loaded through <img> because each part has to be
 * animated separately, which needs real elements in the DOM to target.
 *
 * >>> If the logo changes, scripts/build-logo.py rewrites logo.svg and this
 * >>> copy must be updated to match. logo.svg carries the pointer back here.
 */
export default function Splash({ open }) {
  const t = useT();
  const { mounted, state } = useExitTransition(open, 260);
  if (!mounted) return null;

  return (
    <div className="splash" data-state={state} role="status">
      <svg className="splash__logo" viewBox="0 0 1024 1024" aria-hidden="true">
        <path
          d="M 241.98 183 L 782.02 183 A 55 55 0 0 1 833.73 219.27 L 974.13 607 L 49.87 607 L 190.27 219.27 A 55 55 0 0 1 241.98 183 Z"
          fill="#A8B0BA"
        />
        <path
          className="splash__arrow"
          d="M 465 183 L 559 183 L 559 365 L 660 365 L 512 540 L 364 365 L 465 365 Z"
          fill="#000000"
        />
        <path
          d="M 86 598.5 L 938 598.5 A 40 40 0 0 1 978 638.5 L 978 793.5 A 50 50 0 0 1 928 843.5 L 96 843.5 A 50 50 0 0 1 46 793.5 L 46 638.5 A 40 40 0 0 1 86 598.5 Z"
          fill="#010312"
        />
        <rect x="139" y="701" width="327" height="34" rx="17" fill="#FE511F" />
        <circle className="splash__led splash__led--1" cx="698.5" cy="717" r="27" fill="#02B3FF" />
        <circle className="splash__led splash__led--2" cx="783.5" cy="717" r="27" fill="#60D04A" />
        <circle className="splash__led splash__led--3" cx="868.5" cy="717" r="27" fill="#FC5122" />
      </svg>
      <span className="sr-only">{t('app.loading')}</span>
    </div>
  );
}
