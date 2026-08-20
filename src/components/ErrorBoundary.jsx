import React from 'react';

// A render throw used to blank the window with no way back. Not localized
// on purpose: it has to work when the app — i18n included — is what broke.
export default class ErrorBoundary extends React.Component {
  constructor(props) {
    super(props);
    this.state = { error: null };
  }

  static getDerivedStateFromError(error) {
    return { error };
  }

  componentDidCatch(error, info) {
    console.error('driveby crashed while rendering', error, info);
  }

  render() {
    if (!this.state.error) return this.props.children;
    return (
      <div className="app-crash" role="alert">
        <h1 className="title-2">driveby hit an unexpected error</h1>
        <p>Your tasks, history and settings are stored on disk and are unaffected.</p>
        <pre className="app-crash__detail">{String(this.state.error?.stack || this.state.error)}</pre>
        <button type="button" className="btn btn--primary" onClick={() => window.location.reload()}>
          Reload
        </button>
      </div>
    );
  }
}
