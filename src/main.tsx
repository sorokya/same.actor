import { Component, type ComponentChildren, render } from 'preact';
import { AdminRoot } from './admin/root';
import { App } from './app';
import './index.css';

interface BoundaryState {
  error: Error | null;
}

/**
 * Error boundary. Preact core calls `componentDidCatch` when a render throws, so the
 * boundary re-renders with the error instead of the tree unmounting — no
 * `react-error-boundary`, and therefore no `preact/compat` anywhere in the bundle.
 */
class ErrorBoundary extends Component<{ children: ComponentChildren }, BoundaryState> {
  override state: BoundaryState = { error: null };

  override componentDidCatch(error: Error) {
    this.setState({ error });
  }

  override render() {
    const { error } = this.state;
    if (error === null) return this.props.children;

    return (
      <div className="mx-auto max-w-xl px-4 py-16">
        <h1 className="text-lg text-paper">Something broke.</h1>
        <p className="mt-2 text-sm text-dim">
          The page hit an error it could not recover from. Reloading usually fixes it.
        </p>
        <pre className="mt-4 overflow-x-auto rounded border border-line bg-raise p-3 font-mono text-xs text-danger">
          {error.message}
        </pre>
        <div className="mt-4 flex gap-2">
          <button
            type="button"
            onClick={() => this.setState({ error: null })}
            className="rounded border border-line px-3 py-1.5 text-sm text-paper hover:bg-raise"
          >
            Try again
          </button>
          <button
            type="button"
            onClick={() => window.location.assign('/')}
            className="rounded border border-line px-3 py-1.5 text-sm text-dim hover:text-paper"
          >
            Start over
          </button>
        </div>
      </div>
    );
  }
}

const container = document.getElementById('root');
if (!container) throw new Error('#root is missing from index.html');

// One binary, two surfaces. `/admin` is a separate entry point because it is a separate job:
// the comparison tool is what a visitor opens, and nothing about it should wait on, or pay for,
// the dashboard's chunk.
const isAdmin = window.location.pathname.startsWith('/admin');

render(<ErrorBoundary>{isAdmin ? <AdminRoot /> : <App />}</ErrorBoundary>, container);
