import { useEffect, useState } from 'react';

export function useElapsedSeconds(since: number | null) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (since === null) return;
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [since]);
  return since === null ? null : Math.max(0, Math.floor((now - since) / 1000));
}
