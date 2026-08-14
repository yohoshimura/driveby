import React from 'react';

export default function FormField({ label, hint, htmlFor, children }) {
  return (
    <div className="form-field">
      {label && <label htmlFor={htmlFor} className="form-field__label">{label}</label>}
      {children}
      {hint && <div className="form-field__hint">{hint}</div>}
    </div>
  );
}
