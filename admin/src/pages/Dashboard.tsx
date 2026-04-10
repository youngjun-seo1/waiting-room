import { useCallback, useState } from 'react';
import { api } from '../api';
import { usePolling } from '../hooks/usePolling';
import { StatusBadge } from '../components/StatusBadge';
import { Settings } from '../components/Settings';
import { QueueVisualizer } from '../components/QueueVisualizer';

interface Stats {
  active_users: number;
  queue_length: number;
  avg_active_duration_secs: number;
  total_admitted: number;
}

interface Config {
  max_active_users: number;
  session_ttl_secs: number;
  enabled: boolean;
  origin_url: string;
  redis_url: string;
}

export function Dashboard() {
  const [stats, setStats] = useState<Stats | null>(null);
  const [config, setConfig] = useState<Config | null>(null);
  const [error, setError] = useState('');

  const fetchAll = useCallback(async () => {
    try {
      const [s, c] = await Promise.all([api.getStats(), api.getConfig()]);
      setStats(s);
      setConfig(c);
      setError('');
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : 'Connection lost');
    }
  }, []);

  usePolling(fetchAll, 2000, true);

  return (
    <div className="space-y-6">
      {error && (
        <div className="bg-red-50 text-red-600 text-sm rounded-lg p-3">{error}</div>
      )}

      <QueueVisualizer stats={stats} enabled={config?.enabled ?? false} />

      {config && (
        <>
          <StatusBadge
            enabled={config.enabled}
            maxActive={config.max_active_users}
            sessionTtl={config.session_ttl_secs}
          />
          <div className="flex justify-center">
            <span className="text-xs text-gray-400">
              {config.redis_url ? `Redis: ${config.redis_url}` : 'In-Memory'} | Origin: {config.origin_url}
            </span>
          </div>
        </>
      )}

      <Settings config={config} onRefresh={fetchAll} />
    </div>
  );
}
