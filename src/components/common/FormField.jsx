import React from 'react';
import InfoTip from './InfoTip';

// `tip` explains the field in a bubble beside its label; `hint` is a line
// under the field for what changes as it is filled in, such as the next run.
export default function FormField({ label, tip, hint, htmlFor, children }) {
  return (
    <div className="form-field">
      {label && (
        <div className="form-field__head">
          <label htmlFor={htmlFor} className="form-field__label">{label}</label>
          <InfoTip placement="right" text={tip} />
        </div>
      )}
      {children}
      {hint && <div className="form-field__hint">{hint}</div>}
    </div>
  );
}
