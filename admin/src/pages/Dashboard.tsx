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

interface ScheduleStats {
  peak_active_users: number;
  peak_queue_length: number;
  total_admitted: number;
  total_visitors: number;
}

interface Schedule {
  id: string;
  name: string;
  start_at: string;
  end_at: string;
  max_active_users: number | null;
  origin_url: string | null;
  phase: string;
  stats?: ScheduleStats;
}

function formatTime(iso: string) {
  return new Date(iso).toLocaleString();
}

export function Dashboard() {
  const [stats, setStats] = useState<Stats | null>(null);
  const [config, setConfig] = useState<Config | null>(null);
  const [activeSchedule, setActiveSchedule] = useState<Schedule | null>(null);
  const [error, setError] = useState('');

  const fetchAll = useCallback(async () => {
    try {
      const [s, c, sch] = await Promise.all([
        api.getStats(),
        api.getConfig(),
        api.getSchedules(),
      ]);
      setStats(s);
      setConfig(c);
      const active = (sch.schedules || []).find((s: Schedule) => s.phase === 'active') || null;
      setActiveSchedule(active);
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

          {activeSchedule && (
            <div className="bg-indigo-50 border border-indigo-200 rounded-xl p-4">
              <div className="flex items-center gap-2 mb-2">
                <span className="px-2 py-0.5 rounded-full text-xs font-medium bg-green-100 text-green-700">
                  active
                </span>
                <span className="text-sm font-semibold text-indigo-900">{activeSchedule.name}</span>
              </div>
              <div className="grid grid-cols-2 gap-x-6 gap-y-1 text-xs text-indigo-700">
                <span>Start: {formatTime(activeSchedule.start_at)}</span>
                <span>End: {formatTime(activeSchedule.end_at)}</span>
                {activeSchedule.max_active_users && (
                  <span>Max Active: {activeSchedule.max_active_users}</span>
                )}
                {activeSchedule.origin_url && (
                  <span>Origin: {activeSchedule.origin_url}</span>
                )}
              </div>
              {activeSchedule.stats && (
                <div className="grid grid-cols-4 gap-2 mt-3 pt-3 border-t border-indigo-200">
                  <div className="text-center">
                    <div className="text-lg font-bold text-indigo-900">{activeSchedule.stats.peak_active_users.toLocaleString()}</div>
                    <div className="text-[10px] text-indigo-500">Peak Active</div>
                  </div>
                  <div className="text-center">
                    <div className="text-lg font-bold text-indigo-900">{activeSchedule.stats.peak_queue_length.toLocaleString()}</div>
                    <div className="text-[10px] text-indigo-500">Peak Queue</div>
                  </div>
                  <div className="text-center">
                    <div className="text-lg font-bold text-indigo-900">{activeSchedule.stats.total_admitted.toLocaleString()}</div>
                    <div className="text-[10px] text-indigo-500">Admitted</div>
                  </div>
                  <div className="text-center">
                    <div className="text-lg font-bold text-indigo-900">{activeSchedule.stats.total_visitors.toLocaleString()}</div>
                    <div className="text-[10px] text-indigo-500">Visitors</div>
                  </div>
                </div>
              )}
            </div>
          )}

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
