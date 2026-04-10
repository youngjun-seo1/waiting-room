export function StatusBadge({ enabled, maxActive, sessionTtl }: {
  enabled: boolean;
  maxActive: number;
  sessionTtl: number;
}) {
  return (
    <div className="flex gap-3 justify-center items-center">
      <span className={`px-3 py-1 rounded-full text-sm font-medium ${
        enabled ? 'bg-green-100 text-green-700' : 'bg-gray-100 text-gray-500'
      }`}>
        {enabled ? 'Enabled' : 'Disabled'}
      </span>
      <span className="px-3 py-1 rounded-full text-sm font-medium bg-purple-50 text-purple-700">
        Max Active: {maxActive}
      </span>
      <span className="px-3 py-1 rounded-full text-sm font-medium bg-orange-50 text-orange-700">
        TTL: {sessionTtl}s
      </span>
    </div>
  );
}
