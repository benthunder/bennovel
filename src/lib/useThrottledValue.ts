import { useEffect, useRef, useState } from 'react';

/**
 * Throttles `value`: the first change fires immediately, further changes fire at most
 * once per `wait` ms, and the last value always lands (trailing call).
 */
export function useThrottledValue<T>(value: T, wait: number): T {
  const [throttled, setThrottled] = useState(value);
  const lastFire = useRef(0);

  useEffect(() => {
    const elapsed = Date.now() - lastFire.current;
    if (elapsed >= wait) {
      lastFire.current = Date.now();
      setThrottled(value);
      return;
    }
    const t = setTimeout(() => {
      lastFire.current = Date.now();
      setThrottled(value);
    }, wait - elapsed);
    return () => clearTimeout(t);
  }, [value, wait]);

  return throttled;
}
