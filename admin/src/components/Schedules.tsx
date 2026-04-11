import { memo, useRef, useState } from 'react';
import { api } from '../api';

interface Schedule {
  id: string;
  name: string;
  start_at: string;
  end_at: string;
  max_active_users: number | null;
  origin_url: string | null;
  session_ttl_secs: number | null;
  phase: string;
}

const phaseBadge: Record<string, string> = {
  pending: 'bg-gray-100 text-gray-600',
  active: 'bg-green-100 text-green-700',
  ended: 'bg-red-100 text-red-600',
};

function formatTime(iso: string) {
  return new Date(iso).toLocaleString();
}

const ScheduleList = memo(function ScheduleList({
  schedules,
  onDelete,
}: {
  schedules: Schedule[];
  onDelete: (id: string) => void;
}) {
  if (schedules.length === 0) {
    return <p className="text-sm text-gray-400 mb-4">등록된 스케줄이 없습니다.</p>;
  }

  return (
    <div className="space-y-3 mb-6">
      {schedules.map((s) => (
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
                ttl: {s.session_ttl_secs ?? '?'}s{!s.session_ttl_secs && ' (default)'}
              </span>
            </div>
            <div className="text-xs text-gray-400 mt-1">
              Start: {formatTime(s.start_at)} / End: {formatTime(s.end_at)}{s.origin_url && ` / Origin: ${s.origin_url}`}
            </div>
          </div>
          <button
            onClick={() => onDelete(s.id)}
            className="text-red-500 hover:text-red-700 text-sm"
          >
            Delete
          </button>
        </div>
      ))}
    </div>
  );
});

function ScheduleForm({ onCreated }: { onCreated: () => void }) {
  const nameRef = useRef<HTMLInputElement>(null);
  const startAtRef = useRef<HTMLInputElement>(null);
  const endAtRef = useRef<HTMLInputElement>(null);
  const maxActiveRef = useRef<HTMLInputElement>(null);
  const sessionTtlRef = useRef<HTMLInputElement>(null);
  const [error, setError] = useState('');
  const [creating, setCreating] = useState(false);

  const handleCreate = async () => {
    const name = nameRef.current?.value || '';
    const startAt = startAtRef.current?.value || '';
    const endAt = endAtRef.current?.value || '';
    const maxActive = maxActiveRef.current?.value || '';

    if (!name.trim() || !startAt || !endAt) {
      setError('모든 필드를 입력하세요.');
      return;
    }
    setError('');
    setCreating(true);
    try {
      const data: Record<string, unknown> = {
        name: name.trim(),
        start_at: new Date(startAt).toISOString(),
        end_at: new Date(endAt).toISOString(),
      };
      if (maxActive) data.max_active_users = parseInt(maxActive);
      const sessionTtl = sessionTtlRef.current?.value || '';
      if (sessionTtl) data.session_ttl_secs = parseInt(sessionTtl);
      await api.createSchedule(data);
      if (nameRef.current) nameRef.current.value = '';
      if (startAtRef.current) startAtRef.current.value = '';
      if (endAtRef.current) endAtRef.current.value = '';
      if (maxActiveRef.current) maxActiveRef.current.value = '';
      if (sessionTtlRef.current) sessionTtlRef.current.value = '';
      setError('');
      onCreated();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : 'Failed to create schedule');
    } finally {
      setCreating(false);
    }
  };

  return (
    <div className="border-t pt-4">
      <h3 className="text-sm font-medium mb-3">New Schedule</h3>
      <div className="grid grid-cols-2 gap-3 mb-3">
        <div>
          <label className="block text-xs text-gray-500 mb-1">Name</label>
          <input
            ref={nameRef}
            type="text"
            placeholder="쿠폰 이벤트"
            className="w-full border rounded-lg px-3 py-2 text-sm"
          />
        </div>
        <div>
          <label className="block text-xs text-gray-500 mb-1">Max Active Users</label>
          <input
            ref={maxActiveRef}
            type="number"
            placeholder="100"
            className="w-full border rounded-lg px-3 py-2 text-sm"
          />
        </div>
        <div>
          <label className="block text-xs text-gray-500 mb-1">Session TTL (초)</label>
          <input
            ref={sessionTtlRef}
            type="number"
            placeholder="미입력 시 서버 기본값"
            className="w-full border rounded-lg px-3 py-2 text-sm"
          />
        </div>
        <div>
          <label className="block text-xs text-gray-500 mb-1">Start At (시작)</label>
          <input
            ref={startAtRef}
            type="datetime-local"
            className="w-full border rounded-lg px-3 py-2 text-sm"
          />
        </div>
        <div>
          <label className="block text-xs text-gray-500 mb-1">End At (종료)</label>
          <input
            ref={endAtRef}
            type="datetime-local"
            className="w-full border rounded-lg px-3 py-2 text-sm"
          />
        </div>
      </div>
      {error && (
        <div className="bg-red-50 text-red-600 text-sm rounded-lg p-3 mb-3">{error}</div>
      )}
      <button
        onClick={handleCreate}
        disabled={creating}
        className="px-4 py-2 bg-indigo-600 text-white rounded-lg text-sm hover:bg-indigo-700 disabled:opacity-50"
      >
        {creating ? 'Creating...' : 'Create Schedule'}
      </button>
    </div>
  );
}

export function Schedules({ schedules, onRefresh }: { schedules: Schedule[]; onRefresh: () => void }) {
  const handleDelete = async (id: string) => {
    if (!confirm('이 스케줄을 삭제하시겠습니까?')) return;
    await api.deleteSchedule(id);
    onRefresh();
  };

  return (
    <div className="bg-white rounded-xl border border-gray-200 p-6">
      <h2 className="text-lg font-semibold mb-4">Schedules</h2>
      <ScheduleList schedules={schedules} onDelete={handleDelete} />
      <ScheduleForm onCreated={onRefresh} />
    </div>
  );
}
