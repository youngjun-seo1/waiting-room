import { useEffect, useRef } from 'react';

interface Stats {
  active_users: number;
  queue_length: number;
  total_admitted: number;
}

interface Dot {
  id: number;
  x: number;
  y: number;
  targetX: number;
  targetY: number;
  finalX: number;
  finalY: number;
  state: 'entering' | 'queued' | 'admitting' | 'active' | 'exiting';
  opacity: number;
  progress: number;
}

const COLORS = {
  entering: '#94a3b8',
  queued: '#60a5fa',
  admitting: '#a78bfa',
  active: '#34d399',
  exiting: '#f87171',
};

const CANVAS_W = 720;
const CANVAS_H = 280;
const DOT_R = 3;
const GATE_X = 360;
const QUEUE_START_X = 40;
const ACTIVE_START_X = 420;
const EXIT_X = 700;
const MAX_DOTS = 500; // max visual dots per zone

function lerp(a: number, b: number, t: number) {
  return a + (b - a) * Math.min(t, 1);
}

function randomQueuePos() {
  return {
    x: QUEUE_START_X + Math.random() * (GATE_X - QUEUE_START_X - 40),
    y: 50 + Math.random() * 170,
  };
}

function randomActivePos() {
  return {
    x: ACTIVE_START_X + Math.random() * (EXIT_X - ACTIVE_START_X - 40),
    y: 50 + Math.random() * 170,
  };
}

function scaleCounts(queue: number, active: number): { queued: number; active: number } {
  const total = queue + active;
  if (total <= MAX_DOTS) return { queued: queue, active };
  const scaledQueued = Math.round((queue / total) * MAX_DOTS);
  return { queued: scaledQueued, active: MAX_DOTS - scaledQueued };
}

export function QueueVisualizer({ stats, enabled }: { stats: Stats | null; enabled: boolean }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const dotsRef = useRef<Dot[]>([]);
  const nextId = useRef(0);
  const animFrame = useRef(0);

  // Sync dots to match actual stats counts (scaled)
  useEffect(() => {
    if (!stats) return;
    const dots = dotsRef.current;
    const { queued: targetQueued, active: targetActive } = scaleCounts(
      stats.queue_length,
      stats.active_users,
    );

    // Count only settled dots — transitioning dots are already accounted for
    const currentQueued = dots.filter(d => d.state === 'queued').length;
    const currentEntering = dots.filter(d => d.state === 'entering').length;
    const currentActive = dots.filter(d => d.state === 'active').length;
    const currentAdmitting = dots.filter(d => d.state === 'admitting').length;

    // --- Adjust queued dots ---
    // settled + incoming entering dots should match target
    const effectiveQueued = currentQueued + currentEntering;
    const queueDiff = targetQueued - effectiveQueued;
    if (queueDiff > 0) {
      for (let i = 0; i < queueDiff; i++) {
        const pos = randomQueuePos();
        dots.push({
          id: nextId.current++,
          x: -10,
          y: pos.y,
          targetX: pos.x,
          targetY: pos.y,
          finalX: pos.x,
          finalY: pos.y,
          state: 'entering',
          opacity: 0,
          progress: 0,
        });
      }
    } else if (queueDiff < 0) {
      // Queue shrank → move settled queued dots through gate first
      let toAdmit = Math.min(-queueDiff, currentQueued);
      let moved = 0;
      for (const dot of dots) {
        if (moved >= toAdmit) break;
        if (dot.state === 'queued') {
          const pos = randomActivePos();
          dot.state = 'admitting';
          dot.progress = 0;
          // First move to gate, then to final active position
          dot.targetX = GATE_X;
          dot.targetY = CANVAS_H / 2;
          dot.finalX = pos.x;
          dot.finalY = pos.y;
          moved++;
        }
      }
    }

    // --- Adjust active dots ---
    // settled + incoming admitting dots should match target
    const effectiveActive = currentActive + currentAdmitting;
    const activeDiff = targetActive - effectiveActive;

    if (activeDiff > 0) {
      for (let i = 0; i < activeDiff; i++) {
        const pos = randomActivePos();
        dots.push({
          id: nextId.current++,
          x: GATE_X,
          y: pos.y,
          targetX: pos.x,
          targetY: pos.y,
          finalX: pos.x,
          finalY: pos.y,
          state: 'admitting',
          opacity: 0.5,
          progress: 0,
        });
      }
    } else if (activeDiff < 0) {
      let toRemove = -activeDiff;
      for (const dot of dots) {
        if (toRemove <= 0) break;
        if (dot.state === 'active') {
          dot.state = 'exiting';
          dot.progress = 0;
          dot.targetX = EXIT_X + 20;
          dot.finalX = EXIT_X + 20;
          dot.finalY = dot.y;
          toRemove--;
        }
      }
    }
  }, [stats]);

  // Animation loop
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext('2d')!;

    const animate = () => {
      const dots = dotsRef.current;
      const dpr = window.devicePixelRatio || 1;

      canvas.width = CANVAS_W * dpr;
      canvas.height = CANVAS_H * dpr;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

      ctx.clearRect(0, 0, CANVAS_W, CANVAS_H);

      // Queue zone
      ctx.fillStyle = '#eff6ff';
      ctx.beginPath();
      ctx.roundRect(20, 20, GATE_X - 40, CANVAS_H - 40, 12);
      ctx.fill();

      // Active zone
      ctx.fillStyle = '#ecfdf5';
      ctx.beginPath();
      ctx.roundRect(GATE_X + 20, 20, CANVAS_W - GATE_X - 40, CANVAS_H - 40, 12);
      ctx.fill();

      // Gate line
      ctx.strokeStyle = '#cbd5e1';
      ctx.lineWidth = 2;
      ctx.setLineDash([6, 4]);
      ctx.beginPath();
      ctx.moveTo(GATE_X, 25);
      ctx.lineTo(GATE_X, CANVAS_H - 25);
      ctx.stroke();
      ctx.setLineDash([]);

      // Labels
      ctx.fillStyle = '#94a3b8';
      ctx.font = '11px -apple-system, sans-serif';
      ctx.textAlign = 'center';
      ctx.fillText('GATE', GATE_X, 16);

      ctx.fillStyle = '#60a5fa';
      ctx.font = 'bold 11px -apple-system, sans-serif';
      ctx.textAlign = 'left';
      ctx.fillText('WAITING QUEUE', 30, 38);

      ctx.fillStyle = '#34d399';
      ctx.textAlign = 'right';
      ctx.fillText('ACTIVE', CANVAS_W - 30, 38);

      // Update and draw dots
      for (let i = dots.length - 1; i >= 0; i--) {
        const dot = dots[i];
        dot.progress += 0.025;

        switch (dot.state) {
          case 'entering':
            dot.x = lerp(dot.x, dot.targetX, dot.progress);
            dot.y = lerp(dot.y, dot.targetY, dot.progress);
            dot.opacity = Math.min(dot.opacity + 0.05, 0.85);
            if (dot.progress >= 1) {
              dot.state = 'queued';
              dot.progress = 0;
            }
            break;
          case 'queued':
            dot.x = dot.targetX + Math.sin(Date.now() * 0.001 + dot.id * 0.7) * 2;
            dot.y = dot.targetY + Math.cos(Date.now() * 0.0015 + dot.id * 0.5) * 1.5;
            dot.opacity = 0.7 + Math.sin(Date.now() * 0.002 + dot.id) * 0.15;
            break;
          case 'admitting':
            dot.x = lerp(dot.x, dot.targetX, dot.progress);
            dot.y = lerp(dot.y, dot.targetY, dot.progress);
            dot.opacity = Math.min(dot.opacity + 0.03, 0.9);
            if (dot.progress >= 1) {
              // If at gate (intermediate stop), continue to final active position
              if (dot.targetX !== dot.finalX || dot.targetY !== dot.finalY) {
                dot.targetX = dot.finalX;
                dot.targetY = dot.finalY;
                dot.progress = 0;
              } else {
                dot.state = 'active';
                dot.progress = 0;
              }
            }
            break;
          case 'active':
            dot.x = dot.targetX + Math.sin(Date.now() * 0.0008 + dot.id * 0.3) * 3;
            dot.y = dot.targetY + Math.cos(Date.now() * 0.001 + dot.id * 0.4) * 2;
            dot.opacity = 0.8 + Math.sin(Date.now() * 0.003 + dot.id) * 0.1;
            break;
          case 'exiting':
            dot.x = lerp(dot.x, dot.targetX, dot.progress);
            dot.opacity = Math.max(dot.opacity - 0.04, 0);
            if (dot.opacity <= 0) {
              dots.splice(i, 1);
              continue;
            }
            break;
        }

        const color = COLORS[dot.state];
        ctx.globalAlpha = dot.opacity;
        ctx.fillStyle = color;
        ctx.beginPath();
        ctx.arc(dot.x, dot.y, DOT_R, 0, Math.PI * 2);
        ctx.fill();

        if (dot.state === 'admitting') {
          ctx.globalAlpha = dot.opacity * 0.3;
          ctx.beginPath();
          ctx.arc(dot.x, dot.y, DOT_R * 2.5, 0, Math.PI * 2);
          ctx.fill();
        }
      }

      ctx.globalAlpha = 1;
      animFrame.current = requestAnimationFrame(animate);
    };

    animFrame.current = requestAnimationFrame(animate);
    return () => cancelAnimationFrame(animFrame.current);
  }, []);

  return (
    <div className="relative bg-white rounded-xl border border-gray-200 p-4">
      {!enabled && (
        <div className="absolute inset-0 flex items-center justify-center z-10 rounded-xl bg-white/70 backdrop-blur-sm">
          <span className="text-lg font-semibold text-gray-400">Waiting Room Disabled</span>
        </div>
      )}
      <div className="flex items-center justify-between mb-2">
        <h2 className="text-sm font-semibold text-gray-700">Live Queue Flow</h2>
        <div className="flex gap-4 text-xs text-gray-500">
          <span className="flex items-center gap-1">
            <span className="inline-block w-2 h-2 rounded-full" style={{ background: COLORS.queued }} />
            Waiting
          </span>
          <span className="flex items-center gap-1">
            <span className="inline-block w-2 h-2 rounded-full" style={{ background: COLORS.admitting }} />
            Admitting
          </span>
          <span className="flex items-center gap-1">
            <span className="inline-block w-2 h-2 rounded-full" style={{ background: COLORS.active }} />
            Active
          </span>
          <span className="flex items-center gap-1">
            <span className="inline-block w-2 h-2 rounded-full" style={{ background: COLORS.exiting }} />
            Exiting
          </span>
        </div>
      </div>
      <canvas
        ref={canvasRef}
        style={{ width: CANVAS_W, height: CANVAS_H, display: 'block', margin: '0 auto' }}
      />
      <div className="flex mt-2" style={{ width: CANVAS_W, margin: '0 auto' }}>
        <div className="flex-1 text-center">
          <span className="text-2xl font-bold text-blue-500">{stats?.queue_length ?? 0}</span>
          <span className="text-xs text-gray-400 ml-1">in queue</span>
        </div>
        <div className="flex-1 text-center">
          <span className="text-2xl font-bold text-purple-500">{stats?.total_admitted ?? 0}</span>
          <span className="text-xs text-gray-400 ml-1">admitted</span>
        </div>
        <div className="flex-1 text-center">
          <span className="text-2xl font-bold text-green-500">{stats?.active_users ?? 0}</span>
          <span className="text-xs text-gray-400 ml-1">active</span>
        </div>
      </div>
    </div>
  );
}
