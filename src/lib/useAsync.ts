import { useEffect, useState, type DependencyList } from 'react';

export type AsyncState<T> = { status: 'loading' } | { status: 'ready'; data: T } | { status: 'failed'; error: unknown };

/** Runs `load` whenever `deps` change and ignores results that arrive after a newer run started. */
export function useAsync<T>(load: () => Promise<T>, deps: DependencyList): AsyncState<T> {
  const [state, setState] = useState<AsyncState<T>>({ status: 'loading' });
  useEffect(() => {
    let live = true;
    setState({ status: 'loading' });
    load().then(
      data => { if (live) setState({ status: 'ready', data }); },
      error => { if (live) { console.error(error); setState({ status: 'failed', error }); } }
    );
    return () => { live = false; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);
  return state;
}
