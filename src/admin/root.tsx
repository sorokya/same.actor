/**
 * Loads the dashboard only when someone is actually looking at it.
 *
 * A dynamic `import()` is enough: Vite emits a separate chunk, so a visitor to the comparison
 * tool never downloads the admin code, and Preact's `lazy`/`Suspense` are not needed (they live
 * in `preact/compat`, which this bundle deliberately does not include).
 */

import type { ComponentType } from 'preact';
import { useEffect, useState } from 'preact/hooks';

export function AdminRoot() {
  const [Dashboard, setDashboard] = useState<ComponentType | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let live = true;

    import('./dashboard')
      .then((module) => {
        if (live) setDashboard(() => module.AdminDashboard);
      })
      .catch(() => {
        // Usually a stale chunk after a deploy: the page was loaded before the new build and the
        // old asset name no longer exists. Reloading is the fix.
        if (live) setFailed(true);
      });

    return () => {
      live = false;
    };
  }, []);

  if (failed) {
    return (
      <div className="mx-auto max-w-md px-4 py-16">
        <h1 className="text-sm text-paper">The dashboard did not load.</h1>
        <p className="mt-2 text-xs text-dim">
          This usually means the page was open across a deploy. Reloading fetches the current build.
        </p>
        <button
          type="button"
          onClick={() => window.location.reload()}
          className="mt-4 rounded border border-line px-3 py-1.5 text-xs text-paper hover:bg-raise"
        >
          Reload
        </button>
      </div>
    );
  }

  if (!Dashboard) {
    return <p className="mx-auto max-w-md px-4 py-16 font-mono text-xs text-dim">Loading…</p>;
  }

  return <Dashboard />;
}
