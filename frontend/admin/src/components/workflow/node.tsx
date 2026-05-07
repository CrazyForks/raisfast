
import { memo } from "react";
import { Handle, Position, type Node, type NodeProps } from "@xyflow/react";
import {
  Cog,
  Hourglass,
  GitBranch,
  Layers,
  Timer,
  Pencil,
} from "lucide-react";

export const STEP_META: Record<
  string,
  {
    label: string;
    color: string;
    bg: string;
    Icon: React.ComponentType<{ className?: string; style?: React.CSSProperties }>;
  }
> = {
  task: { label: "Task", color: "#3b82f6", bg: "bg-blue-50", Icon: Cog },
  await: { label: "Await", color: "#f59e0b", bg: "bg-amber-50", Icon: Hourglass },
  branch: { label: "Branch", color: "#8b5cf6", bg: "bg-violet-50", Icon: GitBranch },
  parallel: { label: "Parallel", color: "#10b981", bg: "bg-emerald-50", Icon: Layers },
  delay: { label: "Delay", color: "#6b7280", bg: "bg-gray-50", Icon: Timer },
};

const DEFAULT_META = {
  label: "Step",
  color: "#6b7280",
  bg: "bg-gray-50",
  Icon: Cog as React.ComponentType<{ className?: string; style?: React.CSSProperties }>,
};

export type StepNodeData = {
  title: string;
  stepType: string;
  config: Record<string, unknown>;
  next: unknown;
};

export type StepNode = Node<StepNodeData, "step">;

function StepNodeComponent({ data, id }: NodeProps<StepNode>) {
  const stepType = data.stepType ?? "task";
  const meta = STEP_META[stepType] ?? DEFAULT_META;
  const { Icon } = meta;
  const isBranch = stepType === "branch";
  const branches = isBranch && Array.isArray(data.next)
    ? (data.next as { condition?: Record<string, unknown>; step: string }[])
    : [];

  return (
    <div
      className={`rounded-lg border-2 shadow-sm min-w-[160px] max-w-[260px] select-none group relative ${meta.bg}`}
      style={{ borderColor: meta.color }}
      onDoubleClick={() => {
        window.dispatchEvent(new CustomEvent("workflow:edit-node", { detail: { id } }));
      }}
    >
      <Handle type="target" position={Position.Left} className="!bg-gray-400 !w-2.5 !h-2.5" />

      <div
        className="flex items-center gap-2 px-3 py-2 border-b"
        style={{ borderColor: meta.color + "40" }}
      >
        <Icon className="size-4 shrink-0" style={{ color: meta.color }} />
        <span className="text-xs font-medium text-gray-500">{meta.label}</span>
        <Pencil
          className="size-3 ml-auto opacity-0 group-hover:opacity-50 transition-opacity cursor-pointer nodrag"
          style={{ color: meta.color }}
        />
      </div>
      <div className="px-3 py-2">
        <p className="text-sm font-medium text-gray-900 truncate">{data.title || stepType}</p>
      </div>

      {isBranch && branches.length > 0 ? (
        branches.map((_, i) => {
          const offset = ((i - (branches.length - 1) / 2) / Math.max(branches.length, 1)) * 60;
          return (
            <Handle
              key={i}
              type="source"
              position={Position.Right}
              id={`branch-${i}`}
              className="!bg-violet-400 !w-2.5 !h-2.5"
              style={{ top: "50%", transform: `translateY(${offset}px)` }}
            />
          );
        })
      ) : (
        <Handle type="source" position={Position.Right} className="!bg-gray-400 !w-2.5 !h-2.5" />
      )}
    </div>
  );
}

export const StepNode = memo(StepNodeComponent);
