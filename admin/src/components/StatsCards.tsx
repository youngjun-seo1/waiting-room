interface Stats {
  active_users: number;
  queue_length: number;
  avg_active_duration_secs: number;
}

export function StatsCards({ stats }: { stats: Stats | null }) {
  if (!stats) return null;

  const cards = [
    { label: 'Active Users', value: stats.active_users, color: 'text-green-600', bg: 'bg-green-50' },
    { label: 'Queue Length', value: stats.queue_length, color: 'text-blue-600', bg: 'bg-blue-50' },
    { label: 'Avg Duration', value: `${stats.avg_active_duration_secs.toFixed(1)}s`, color: 'text-purple-600', bg: 'bg-purple-50' },
  ];

  return (
    <div className="grid grid-cols-3 gap-4">
      {cards.map((card) => (
        <div key={card.label} className={`${card.bg} rounded-xl p-6 text-center`}>
          <div className={`text-4xl font-bold ${card.color}`}>{card.value}</div>
          <div className="text-sm text-gray-500 mt-2">{card.label}</div>
        </div>
      ))}
    </div>
  );
}
