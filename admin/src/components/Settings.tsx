import { useState } from 'react';
import { api } from '../api';

interface Config {
  max_active_users: number;
  session_ttl_secs: number;
  enabled: boolean;
}

export function Settings({ config, onRefresh }: { config: Config | null; onRefresh: () => void }) {
  const [maxActive, setMaxActive] = useState('');
  const [ttl, setTtl] = useState('');

  if (!config) return null;

  const handleSave = async () => {
    const update: Record<string, unknown> = {};
    if (maxActive) update.max_active_users = parseInt(maxActive);
    if (ttl) update.session_ttl_secs = parseInt(ttl);
    if (Object.keys(update).length === 0) return;
    await api.updateConfig(update);
    setMaxActive('');
    setTtl('');
    onRefresh();
  };

  const handleToggle = async () => {
    if (config.enabled) {
      await api.disable();
    } else {
      await api.enable();
    }
    onRefresh();
  };

  const handleFlush = async () => {
    if (!confirm('Queue를 초기화하시겠습니까? 대기 중인 모든 사용자가 제거됩니다.')) return;
    await api.flush();
    onRefresh();
  };

  return (
    <div className="bg-white rounded-xl border border-gray-200 p-6">
      <h2 className="text-lg font-semibold mb-4">Settings</h2>
      <div className="grid grid-cols-2 gap-4 mb-4">
        <div>
          <label className="block text-sm text-gray-500 mb-1">Max Active Users</label>
          <input
            type="number"
            placeholder={String(config.max_active_users)}
            value={maxActive}
            onChange={(e) => setMaxActive(e.target.value)}
            className="w-full border rounded-lg px-3 py-2 text-sm"
          />
        </div>
        <div>
          <label className="block text-sm text-gray-500 mb-1">Session TTL (sec)</label>
          <input
            type="number"
            placeholder={String(config.session_ttl_secs)}
            value={ttl}
            onChange={(e) => setTtl(e.target.value)}
            className="w-full border rounded-lg px-3 py-2 text-sm"
          />
        </div>
      </div>
      <div className="flex gap-3">
        <button
          onClick={handleSave}
          className="px-4 py-2 bg-indigo-600 text-white rounded-lg text-sm hover:bg-indigo-700"
        >
          Save
        </button>
        <button
          onClick={handleToggle}
          className={`px-4 py-2 rounded-lg text-sm text-white ${
            config.enabled ? 'bg-orange-500 hover:bg-orange-600' : 'bg-green-600 hover:bg-green-700'
          }`}
        >
          {config.enabled ? 'Disable' : 'Enable'}
        </button>
        <button
          onClick={handleFlush}
          className="px-4 py-2 bg-red-500 text-white rounded-lg text-sm hover:bg-red-600"
        >
          Flush Queue
        </button>
      </div>
    </div>
  );
}
