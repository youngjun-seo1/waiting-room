import { useCallback, useEffect, useState } from 'react';
import { api } from '../api';
import { usePolling } from '../hooks/usePolling';
import { QueueVisualizer } from '../components/QueueVisualizer';

function LiveClock() {
  const [now, setNow] = useState(new Date());
  useEffect(() => {
    const id = setInterval(() => setNow(new Date()), 1000);
    return () => clearInterval(id);
  }, []);
  const time = now.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false });
  const date = now.toLocaleDateString([], { year: 'numeric', month: '2-digit', day: '2-digit', weekday: 'short' });
  return (
    <div className="flex items-baseline gap-3">
      <span className="text-2xl font-mono font-bold tracking-widest text-gray-800 tabular-nums">{time}</span>
      <span className="text-xs text-gray-400">{date}</span>
    </div>
  );
}

interface Status {
  enabled: boolean;
  active_users: number;
  queue_length: number;
  total_admitted: number;
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
  session_ttl_secs: number | null;
  phase: string;
  stats?: ScheduleStats;
}

function formatTime(iso: string) {
  return new Date(iso).toLocaleString();
}

function formatRange(startIso: string, endIso: string) {
  const start = new Date(startIso);
  const end = new Date(endIso);
  const sameDay = start.toLocaleDateString() === end.toLocaleDateString();
  if (sameDay) {
    return `${start.toLocaleDateString()} ${start.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })} ~ ${end.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`;
  }
  return `${formatTime(startIso)} ~ ${formatTime(endIso)}`;
}

export function Dashboard() {
  const [status, setStatus] = useState<Status | null>(null);
const [activeSchedule, setActiveSchedule] = useState<Schedule | null>(null);
  const [lastEndedSchedule, setLastEndedSchedule] = useState<Schedule | null>(null);
  const [nextPendingSchedule, setNextPendingSchedule] = useState<Schedule | null>(null);
  const [error, setError] = useState('');

  const fetchAll = useCallback(async () => {
    try {
      const [st, sch] = await Promise.all([
        api.getStatus(),
        api.getSchedules(),
      ]);
      setStatus(st);
      const schedules: Schedule[] = sch.schedules || [];
      const active = schedules.find((s) => s.phase === 'active') || null;
      setActiveSchedule(active);

      const ended = schedules
        .filter((s) => s.phase === 'ended')
        .sort((a, b) => new Date(b.end_at).getTime() - new Date(a.end_at).getTime());
      setLastEndedSchedule(ended[0] || null);

      const pending = schedules
        .filter((s) => s.phase === 'pending')
        .sort((a, b) => new Date(a.start_at).getTime() - new Date(b.start_at).getTime());
      setNextPendingSchedule(pending[0] || null);

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

      <div className="flex items-center justify-between">
        <LiveClock />
        <span className={`inline-flex items-center gap-1.5 text-xs font-medium ${status?.enabled ? 'text-green-600' : 'text-gray-400'}`}>
          <span className={`w-1.5 h-1.5 rounded-full ${status?.enabled ? 'bg-green-500 animate-pulse' : 'bg-gray-300'}`} />
          {status?.enabled ? 'LIVE' : 'OFFLINE'}
        </span>
      </div>

      <QueueVisualizer stats={status} enabled={status?.enabled ?? false} />

      {/* Active Schedule */}
      {activeSchedule && (
        <div className="bg-indigo-50 border border-indigo-200 rounded-xl p-4">
          <div className="flex items-center gap-2 mb-2">
            <span className="px-2 py-0.5 rounded-full text-xs font-medium bg-green-100 text-green-700">
              active
            </span>
            <span className="text-sm font-semibold text-indigo-900">{activeSchedule.name}</span>
            <span className="text-xs text-indigo-400">#{activeSchedule.id}</span>
          </div>
          <div className="flex flex-wrap gap-x-6 gap-y-1 text-xs text-indigo-700">
            <span>{formatRange(activeSchedule.start_at, activeSchedule.end_at)}</span>
            {activeSchedule.max_active_users && (
              <span>Max Active: {activeSchedule.max_active_users}</span>
            )}
            {activeSchedule.session_ttl_secs && (
              <span>TTL: {activeSchedule.session_ttl_secs}s</span>
            )}
            <span>Origin: {activeSchedule.origin_url ?? 'null'}</span>
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

      {/* Recent & Upcoming Schedules */}
      {(lastEndedSchedule || nextPendingSchedule) && (
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
          {/* Last Ended */}
          {lastEndedSchedule && (
            <div className="bg-gray-50 border border-gray-200 rounded-xl p-4">
              <div className="flex items-center gap-2 mb-2">
                <span className="px-2 py-0.5 rounded-full text-xs font-medium bg-red-100 text-red-600">
                  ended
                </span>
                <span className="text-sm font-semibold text-gray-800">{lastEndedSchedule.name}</span>
                <span className="text-xs text-gray-400">#{lastEndedSchedule.id}</span>
              </div>
              <div className="flex flex-wrap gap-x-6 gap-y-1 text-xs text-gray-600">
                <span>{formatRange(lastEndedSchedule.start_at, lastEndedSchedule.end_at)}</span>
                {lastEndedSchedule.max_active_users && (
                  <span>Max Active: {lastEndedSchedule.max_active_users}</span>
                )}
                {lastEndedSchedule.session_ttl_secs && (
                  <span>TTL: {lastEndedSchedule.session_ttl_secs}s</span>
                )}
                <span>Origin: {lastEndedSchedule.origin_url ?? 'null'}</span>
              </div>
              {lastEndedSchedule.stats && (
                <div className="grid grid-cols-4 gap-2 mt-3 pt-3 border-t border-gray-200">
                  <div className="text-center">
                    <div className="text-lg font-bold text-gray-800">{lastEndedSchedule.stats.peak_active_users.toLocaleString()}</div>
                    <div className="text-[10px] text-gray-400">Peak Active</div>
                  </div>
                  <div className="text-center">
                    <div className="text-lg font-bold text-gray-800">{lastEndedSchedule.stats.peak_queue_length.toLocaleString()}</div>
                    <div className="text-[10px] text-gray-400">Peak Queue</div>
                  </div>
                  <div className="text-center">
                    <div className="text-lg font-bold text-gray-800">{lastEndedSchedule.stats.total_admitted.toLocaleString()}</div>
                    <div className="text-[10px] text-gray-400">Admitted</div>
                  </div>
                  <div className="text-center">
                    <div className="text-lg font-bold text-gray-800">{lastEndedSchedule.stats.total_visitors.toLocaleString()}</div>
                    <div className="text-[10px] text-gray-400">Visitors</div>
                  </div>
                </div>
              )}
            </div>
          )}

          {/* Next Pending */}
          {nextPendingSchedule && (
            <div className="bg-yellow-50 border border-yellow-200 rounded-xl p-4">
              <div className="flex items-center gap-2 mb-2">
                <span className="px-2 py-0.5 rounded-full text-xs font-medium bg-yellow-100 text-yellow-700">
                  pending
                </span>
                <span className="text-sm font-semibold text-yellow-900">{nextPendingSchedule.name}</span>
                <span className="text-xs text-yellow-500">#{nextPendingSchedule.id}</span>
              </div>
              <div className="flex flex-wrap gap-x-6 gap-y-1 text-xs text-yellow-700">
                <span>{formatRange(nextPendingSchedule.start_at, nextPendingSchedule.end_at)}</span>
                {nextPendingSchedule.max_active_users && (
                  <span>Max Active: {nextPendingSchedule.max_active_users}</span>
                )}
                {nextPendingSchedule.session_ttl_secs && (
                  <span>TTL: {nextPendingSchedule.session_ttl_secs}s</span>
                )}
                <span>Origin: {nextPendingSchedule.origin_url ?? 'null'}</span>
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
