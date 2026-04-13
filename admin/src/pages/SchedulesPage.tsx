import React, { useCallback, useEffect, useState } from 'react';
import { api } from '../api';

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

const phaseBadge: Record<string, string> = {
  pending: 'bg-gray-100 text-gray-600',
  active: 'bg-green-100 text-green-700',
  ended: 'bg-red-100 text-red-600',
};

function formatTime(iso: string) {
  return new Date(iso).toLocaleString();
}

function formatDuration(startIso: string, endIso: string): string {
  const ms = new Date(endIso).getTime() - new Date(startIso).getTime();
  if (ms <= 0) return '0min';
  const totalMin = Math.floor(ms / 60000);
  const h = Math.floor(totalMin / 60);
  const m = totalMin % 60;
  if (h > 0 && m > 0) return `${h}h ${m}min`;
  if (h > 0) return `${h}h`;
  return `${m}min`;
}

/** "YYYY-MM-DDTHH:MM" for datetime-local input */
function toLocalInput(date: Date): string {
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

function nowLocal() {
  return toLocalInput(new Date());
}

export function SchedulesPage() {
  const [schedules, setSchedules] = useState<Schedule[]>([]);
  const [error, setError] = useState('');
  const [creating, setCreating] = useState(false);
  const [message, setMessage] = useState('');

  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(3);

  const [name, setName] = useState('');
  const [startAt, setStartAt] = useState(nowLocal);
  const [endAt, setEndAt] = useState(nowLocal);
  const [maxActive, setMaxActive] = useState('');
  const [originUrl, setOriginUrl] = useState('');
  const [sessionTtl, setSessionTtl] = useState('');

  const [defaultMaxActive, setDefaultMaxActive] = useState('100');
  const [defaultSessionTtl, setDefaultSessionTtl] = useState('');
  const [defaultOriginUrl, setDefaultOriginUrl] = useState('');

  const fetchSchedules = useCallback(async () => {
    try {
      const res = await api.getSchedules();
      const list: Schedule[] = res.schedules || [];
      setSchedules(list);
      setPage((p) => {
        const maxPage = Math.max(1, Math.ceil(list.length / pageSize));
        return p > maxPage ? maxPage : p;
      });
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : 'Failed to load schedules');
    }
  }, [pageSize]);

  useEffect(() => {
    fetchSchedules();
    const timer = setInterval(fetchSchedules, 5000);
    return () => clearInterval(timer);
  }, [fetchSchedules]);

  useEffect(() => {
    api.getConfig().then((cfg: Record<string, unknown>) => {
      const max = String(cfg.max_active_users ?? '100');
      const ttl = String(cfg.session_ttl_secs ?? '');
      const origin = String(cfg.origin_url ?? '');
      setDefaultMaxActive(max);
      setDefaultSessionTtl(ttl);
      setDefaultOriginUrl(origin);
      setMaxActive(max);
      setSessionTtl(ttl);
    }).catch(() => {});
  }, []);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();

    const trimmedName = name.trim();
    if (!trimmedName || !startAt || !endAt) {
      setError('모든 필수 필드를 입력하세요.');
      return;
    }

    const startIso = new Date(startAt).toISOString();
    const endIso = new Date(endAt).toISOString();

    if (startIso >= endIso) {
      setError('종료 시간은 시작 시간보다 미래여야 합니다.');
      return;
    }

    setError('');
    setCreating(true);
    try {
      const data: Record<string, unknown> = {
        name: trimmedName,
        start_at: startIso,
        end_at: endIso,
      };
      if (maxActive) data.max_active_users = parseInt(maxActive);
      if (originUrl.trim()) data.origin_url = originUrl.trim();
      if (sessionTtl) data.session_ttl_secs = parseInt(sessionTtl);
      await api.createSchedule(data);

      setName('');
      setStartAt(nowLocal());
      setEndAt(nowLocal());
      setMaxActive(defaultMaxActive);
      setOriginUrl('');
      setSessionTtl(defaultSessionTtl);
      setMessage('스케줄이 등록되었습니다.');
      setTimeout(() => setMessage(''), 3000);
      fetchSchedules();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : 'Failed to create schedule');
    } finally {
      setCreating(false);
    }
  };

  const [qtMaxActive, setQtMaxActive] = useState('10');
  const [qtTtl, setQtTtl] = useState('30');
  const [qtDuration, setQtDuration] = useState('5');
  const [qtMessage, setQtMessage] = useState('');
  const [qtError, setQtError] = useState('');

  const handleQuickTest = async (overrideMaxActive?: string, overrideTtl?: string, overrideDuration?: string) => {
    const ma = parseInt(overrideMaxActive ?? qtMaxActive) || 10;
    const ttl = parseInt(overrideTtl ?? qtTtl) || 30;
    const dur = parseInt(overrideDuration ?? qtDuration) || 5;
    setQtError('');
    setCreating(true);
    try {
      const now = new Date();
      const end = new Date(now.getTime() + dur * 60 * 1000);
      const data: Record<string, unknown> = {
        name: `Quick Test (${ma} users, ${ttl}s ttl, ${dur}min)`,
        start_at: now.toISOString(),
        end_at: end.toISOString(),
        max_active_users: ma,
        session_ttl_secs: ttl,
      };
      if (defaultOriginUrl) data.origin_url = defaultOriginUrl;
      await api.createSchedule(data);
      setQtMessage(`테스트 스케줄 생성: ${ma}명, TTL ${ttl}초, ${dur}분간`);
      setTimeout(() => setQtMessage(''), 3000);
      fetchSchedules();
    } catch (e: unknown) {
      setQtError(e instanceof Error ? e.message : 'Failed to create quick test');
    } finally {
      setCreating(false);
    }
  };

  const handleDelete = async (id: string, scheduleName: string) => {
    if (!confirm(`"${scheduleName}" 스케줄을 삭제하시겠습니까?`)) return;
    try {
      await api.deleteSchedule(id);
      fetchSchedules();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : 'Failed to delete schedule');
    }
  };

  return (
    <div className="space-y-6">
      {/* Schedule List */}
      <div className="bg-white rounded-xl border border-gray-200 p-6">
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-lg font-semibold">Registered Schedules</h2>
          <button
            onClick={fetchSchedules}
            className="text-sm text-indigo-600 hover:text-indigo-800"
          >
            Refresh
          </button>
        </div>

        {schedules.length === 0 ? (
          <p className="text-sm text-gray-400">등록된 스케줄이 없습니다.</p>
        ) : (
          <>
            <div className="space-y-3">
              {schedules.slice((page - 1) * pageSize, page * pageSize).map((s) => (
                <div key={s.id} className="flex items-center justify-between border rounded-lg p-4">
                  <div className="text-left">
                    <div className="flex items-center gap-2">
                      <span className="font-medium">{s.name}</span>
                      <span className="text-xs text-gray-400">#{s.id}</span>
                      <span className={`px-2 py-0.5 rounded-full text-xs font-medium ${phaseBadge[s.phase] || 'bg-gray-100'}`}>
                        {s.phase}
                      </span>
                      {s.max_active_users && (
                        <span className="text-xs text-gray-400">max: {s.max_active_users}</span>
                      )}
                      <span className="text-xs text-gray-400">
                        ttl: {s.session_ttl_secs ?? defaultSessionTtl}s{!s.session_ttl_secs && ' (default)'}
                      </span>
                    </div>
                    <div className="text-xs text-gray-400 mt-1">
                      <span>{formatTime(s.start_at)} ~ {formatTime(s.end_at)}</span>
                      <span className="ml-2 text-gray-500 font-medium">({formatDuration(s.start_at, s.end_at)})</span>
                    </div>
                    {s.origin_url && (
                      <div className="text-xs text-gray-400 mt-0.5 truncate">
                        Origin: {s.origin_url}
                      </div>
                    )}
                    {s.stats && s.stats.total_admitted > 0 && (
                      <div className="flex gap-4 text-xs text-gray-500 mt-1">
                        <span>Peak Active: {s.stats.peak_active_users.toLocaleString()}</span>
                        <span>Peak Queue: {s.stats.peak_queue_length.toLocaleString()}</span>
                        <span>Admitted: {s.stats.total_admitted.toLocaleString()}</span>
                        <span>Visitors: {s.stats.total_visitors.toLocaleString()}</span>
                      </div>
                    )}
                  </div>
                  <button
                    onClick={() => handleDelete(s.id, s.name)}
                    className="text-red-500 hover:text-red-700 text-sm px-3 py-1"
                  >
                    Delete
                  </button>
                </div>
              ))}
            </div>

            {/* Pagination */}
            <div className="flex items-center justify-between mt-4 pt-4 border-t border-gray-100">
              <div className="flex items-center gap-2 text-sm text-gray-500">
                <span>Rows</span>
                <select
                  value={pageSize}
                  onChange={(e) => { setPageSize(Number(e.target.value)); setPage(1); }}
                  className="border border-gray-300 rounded px-2 py-1 text-sm"
                >
                  <option value={3}>3</option>
                  <option value={10}>10</option>
                  <option value={100}>100</option>
                </select>
                <span className="text-gray-400">Total {schedules.length}</span>
              </div>
              <div className="flex items-center gap-1">
                <button
                  onClick={() => setPage((p) => Math.max(1, p - 1))}
                  disabled={page <= 1}
                  className="px-3 py-1 text-sm rounded border border-gray-300 disabled:opacity-30 hover:bg-gray-50"
                >
                  Prev
                </button>
                <span className="px-3 py-1 text-sm text-gray-600">
                  {page} / {Math.max(1, Math.ceil(schedules.length / pageSize))}
                </span>
                <button
                  onClick={() => setPage((p) => Math.min(Math.ceil(schedules.length / pageSize), p + 1))}
                  disabled={page >= Math.ceil(schedules.length / pageSize)}
                  className="px-3 py-1 text-sm rounded border border-gray-300 disabled:opacity-30 hover:bg-gray-50"
                >
                  Next
                </button>
              </div>
            </div>
          </>
        )}
      </div>

      {/* Create Schedule Form */}
      <form onSubmit={handleSubmit} className="bg-white rounded-xl border border-gray-200 p-6">
        <h2 className="text-lg font-semibold mb-4">New Schedule</h2>

        <div className="grid grid-cols-3 gap-4 mb-4">
          <div>
            <label className="block text-sm text-gray-600 mb-1">Name *</label>
            <input
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="쿠폰 이벤트"
              className="w-full border border-gray-300 rounded-lg px-3 py-2 text-sm focus:ring-2 focus:ring-indigo-500 focus:border-transparent outline-none"
            />
          </div>
          <div>
            <label className="block text-sm text-gray-600 mb-1">Max Active Users</label>
            <input
              type="number"
              value={maxActive}
              onChange={(e) => setMaxActive(e.target.value)}
              placeholder="100 (optional)"
              className="w-full border border-gray-300 rounded-lg px-3 py-2 text-sm focus:ring-2 focus:ring-indigo-500 focus:border-transparent outline-none"
            />
          </div>
          <div>
            <label className="block text-sm text-gray-600 mb-1">Session TTL (초)</label>
            <input
              type="number"
              value={sessionTtl}
              onChange={(e) => setSessionTtl(e.target.value)}
              placeholder="미입력 시 서버 기본값"
              className="w-full border border-gray-300 rounded-lg px-3 py-2 text-sm focus:ring-2 focus:ring-indigo-500 focus:border-transparent outline-none"
            />
          </div>
        </div>

        <div className="mb-4">
          <label className="block text-sm text-gray-600 mb-1">Origin URL</label>
          <input
            type="text"
            value={originUrl}
            onChange={(e) => setOriginUrl(e.target.value)}
            placeholder="http://localhost:3000 (미입력 시 기본 Origin 사용)"
            className="w-full border border-gray-300 rounded-lg px-3 py-2 text-sm focus:ring-2 focus:ring-indigo-500 focus:border-transparent outline-none"
          />
        </div>

        <div className="grid grid-cols-2 gap-4 mb-4">
          <div>
            <label className="block text-sm text-gray-600 mb-1">Start At * (시작)</label>
            <input
              type="datetime-local"
              value={startAt}
              onChange={(e) => setStartAt(e.target.value)}
              className="w-full border border-gray-300 rounded-lg px-3 py-2 text-sm focus:ring-2 focus:ring-indigo-500 focus:border-transparent outline-none"
            />
          </div>
          <div>
            <label className="block text-sm text-gray-600 mb-1">End At * (종료)</label>
            <input
              type="datetime-local"
              value={endAt}
              onChange={(e) => setEndAt(e.target.value)}
              className="w-full border border-gray-300 rounded-lg px-3 py-2 text-sm focus:ring-2 focus:ring-indigo-500 focus:border-transparent outline-none"
            />
          </div>
        </div>

        {error && (
          <div className="bg-red-50 text-red-600 text-sm rounded-lg p-3 mb-4">{error}</div>
        )}
        {message && (
          <div className="bg-green-50 text-green-600 text-sm rounded-lg p-3 mb-4">{message}</div>
        )}

        <button
          type="submit"
          disabled={creating}
          className="px-6 py-2.5 bg-indigo-600 text-white rounded-lg text-sm font-medium hover:bg-indigo-700 disabled:opacity-50"
        >
          {creating ? 'Creating...' : 'Create Schedule'}
        </button>
      </form>

      {/* Quick Test Card */}
      <div className="bg-amber-50 rounded-xl border border-amber-200 p-6">
        <h2 className="text-lg font-semibold text-amber-800 mb-4">Quick Test</h2>

        <div className="grid grid-cols-3 gap-4 mb-4">
          <div>
            <label className="block text-sm text-amber-700 mb-1">Max Active Users</label>
            <input
              type="number"
              value={qtMaxActive}
              onChange={(e) => setQtMaxActive(e.target.value)}
              min="1"
              className="w-full border border-amber-300 rounded-lg px-3 py-2 text-sm bg-white focus:ring-2 focus:ring-amber-400 focus:border-transparent outline-none"
            />
          </div>
          <div>
            <label className="block text-sm text-amber-700 mb-1">Session TTL (초)</label>
            <input
              type="number"
              value={qtTtl}
              onChange={(e) => setQtTtl(e.target.value)}
              min="1"
              className="w-full border border-amber-300 rounded-lg px-3 py-2 text-sm bg-white focus:ring-2 focus:ring-amber-400 focus:border-transparent outline-none"
            />
          </div>
          <div>
            <label className="block text-sm text-amber-700 mb-1">Duration (분)</label>
            <input
              type="number"
              value={qtDuration}
              onChange={(e) => setQtDuration(e.target.value)}
              min="1"
              className="w-full border border-amber-300 rounded-lg px-3 py-2 text-sm bg-white focus:ring-2 focus:ring-amber-400 focus:border-transparent outline-none"
            />
          </div>
        </div>

        {qtError && (
          <div className="bg-red-50 text-red-600 text-sm rounded-lg p-3 mb-4">{qtError}</div>
        )}
        {qtMessage && (
          <div className="bg-green-50 text-green-600 text-sm rounded-lg p-3 mb-4">{qtMessage}</div>
        )}

        <div className="flex gap-2 flex-wrap">
          <button
            type="button"
            onClick={() => handleQuickTest()}
            disabled={creating}
            className="px-4 py-2 bg-amber-500 text-white rounded-lg text-sm font-medium hover:bg-amber-600 disabled:opacity-50"
          >
            Start Test
          </button>
          <button
            type="button"
            onClick={() => handleQuickTest('10', '5', '5')}
            disabled={creating}
            className="px-4 py-2 bg-teal-500 text-white rounded-lg text-sm font-medium hover:bg-teal-600 disabled:opacity-50"
          >
            Small (10/5s/5min)
          </button>
          <button
            type="button"
            onClick={() => handleQuickTest('100', '10', '5')}
            disabled={creating}
            className="px-4 py-2 bg-sky-500 text-white rounded-lg text-sm font-medium hover:bg-sky-600 disabled:opacity-50"
          >
            Normal (100/10s/5min)
          </button>
          <button
            type="button"
            onClick={() => handleQuickTest('1000', '20', '10')}
            disabled={creating}
            className="px-4 py-2 bg-violet-500 text-white rounded-lg text-sm font-medium hover:bg-violet-600 disabled:opacity-50"
          >
            Large (1000/20s/10min)
          </button>
        </div>
      </div>
    </div>
  );
}
