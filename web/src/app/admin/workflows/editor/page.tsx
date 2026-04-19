"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import {
  ReactFlow,
  ReactFlowProvider,
  Background,
  Controls,
  MiniMap,
  useNodesState,
  useEdgesState,
  addEdge,
  type Connection,
  type Edge,
  type Node,
  BackgroundVariant,
  useReactFlow,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { toast } from "sonner";
import {
  ArrowLeft,
  Save,
  Cog,
  Hourglass,
  GitBranch,
  Layers,
  Timer,
  Trash2,
  Plus,
} from "lucide-react";
import Link from "next/link";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import {
  Select,
  SelectGroup,
  SelectValue,
  SelectItem,
  SelectContent,
  SelectTrigger,
} from "@/components/ui/select";
import { api, ApiError } from "@/lib/api";
import { StepNode, STEP_META, type StepNodeData } from "@/components/workflow/node";

const STEP_COLORS: Record<string, string> = {
  task: "#3b82f6",
  await: "#f59e0b",
  branch: "#8b5cf6",
  parallel: "#10b981",
  delay: "#6b7280",
};

const STEP_TYPES = [
  { type: "task", label: "Task", Icon: Cog, color: STEP_COLORS.task },
  { type: "await", label: "Await", Icon: Hourglass, color: STEP_COLORS.await },
  { type: "branch", label: "Branch", Icon: GitBranch, color: STEP_COLORS.branch },
  { type: "parallel", label: "Parallel", Icon: Layers, color: STEP_COLORS.parallel },
  { type: "delay", label: "Delay", Icon: Timer, color: STEP_COLORS.delay },
] as const;

const NODE_TYPES = { step: StepNode };

function stepsToNodes(
  steps: { id: string; name: string; type: string; config: Record<string, unknown>; next: unknown }[]
): { nodes: Node<StepNodeData>[]; edges: Edge[] } {
  const nodes: Node<StepNodeData>[] = [];
  const edges: Edge[] = [];
  const spacing = 280;
  let x = 100;
  const y = 200;

  for (let i = 0; i < steps.length; i++) {
    const step = steps[i];
    nodes.push({
      id: step.id,
      type: "step",
      position: { x, y },
      data: { title: step.name, stepType: step.type, config: step.config, next: step.next },
    });

    if (i > 0) {
      edges.push({ id: `e-${steps[i - 1].id}-${step.id}`, source: steps[i - 1].id, target: step.id });
    }

    if (step.type === "branch" && Array.isArray(step.next)) {
      const branches = step.next as { condition?: Record<string, unknown>; step: string }[];
      let branchY = y - (branches.length * 120) / 2;
      for (let bi = 0; bi < branches.length; bi++) {
        const branch = branches[bi];
        const branchId = `${step.id}_branch_${bi}`;
        nodes.push({
          id: branchId,
          type: "step",
          position: { x: x + spacing, y: branchY },
          data: {
            title: branch.condition ? JSON.stringify(branch.condition) : "default",
            stepType: "task",
            config: {},
            next: branch.step,
          },
        });
        edges.push({
          id: `e-${step.id}-${branchId}`,
          source: step.id,
          sourceHandle: `branch-${bi}`,
          target: branchId,
        });
        branchY += 120;
      }
    }

    x += spacing;
  }

  return { nodes, edges };
}

function nodesToSteps(nodes: Node<StepNodeData>[]) {
  const sorted = [...nodes]
    .filter((n) => !n.id.includes("_branch_"))
    .sort((a, b) => a.position.x - b.position.x);

  return sorted.map((node) => ({
    id: node.id,
    name: node.data.title || node.id,
    type: node.data.stepType || "task",
    config: node.data.config || {},
    next: node.data.next || "",
  }));
}

function NodeEditSheet({
  node,
  open,
  onOpenChange,
  onSave,
  onDelete,
}: {
  node: Node<StepNodeData> | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSave: (id: string, data: Partial<StepNodeData>) => void;
  onDelete: (id: string) => void;
}) {
  const [title, setTitle] = useState("");
  const [stepType, setStepType] = useState("task");
  const [configJson, setConfigJson] = useState("{}");

  useEffect(() => {
    if (node) {
      setTitle(node.data.title ?? "");
      setStepType(node.data.stepType ?? "task");
      setConfigJson(JSON.stringify(node.data.config ?? {}, null, 2));
    }
  }, [node]);

  if (!node) return null;

  const meta = STEP_META[stepType] ?? STEP_META.task;

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent side="right" className="w-80">
        <SheetHeader>
          <SheetTitle>Edit Step</SheetTitle>
        </SheetHeader>
        <div className="space-y-4 px-1 py-4">
          <div className="space-y-2">
            <Label>Name</Label>
            <Input
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder="Step name"
            />
          </div>
          <div className="space-y-2">
            <Label>Type</Label>
            <Select value={stepType} onValueChange={(v) => { if (v) setStepType(v); }}>
              <SelectTrigger className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectGroup>
                  {STEP_TYPES.map(({ type, label, Icon, color }) => (
                    <SelectItem key={type} value={type}>
                      <div className="flex items-center gap-2">
                        <Icon className="size-4" style={{ color }} />
                        {label}
                      </div>
                    </SelectItem>
                  ))}
                </SelectGroup>
              </SelectContent>
            </Select>
          </div>
          {stepType === "branch" && (
            <div className="space-y-2">
              <Label>Branches (JSON array)</Label>
              <Textarea
                rows={4}
                className="font-mono text-xs"
                value={typeof node.data.next === "string" ? String(node.data.next) : JSON.stringify(node.data.next ?? [], null, 2)}
                disabled
              />
              <p className="text-xs text-muted-foreground">
                Connect edges from this node to define branches.
              </p>
            </div>
          )}
          <div className="space-y-2">
            <Label>Config (JSON)</Label>
            <Textarea
              rows={6}
              className="font-mono text-xs"
              value={configJson}
              onChange={(e) => setConfigJson(e.target.value)}
            />
          </div>
          <div className="flex gap-2 pt-2">
            <Button
              className="flex-1"
              onClick={() => {
                try {
                  const config = JSON.parse(configJson);
                  onSave(node.id, { title, stepType, config });
                  onOpenChange(false);
                } catch {
                  toast.error("Invalid JSON in config");
                }
              }}
              style={{ backgroundColor: meta.color }}
            >
              Apply
            </Button>
            <Button
              variant="destructive"
              size="icon"
              onClick={() => {
                onDelete(node.id);
                onOpenChange(false);
              }}
              title="Delete step"
            >
              <Trash2 className="size-4" />
            </Button>
          </div>
        </div>
      </SheetContent>
    </Sheet>
  );
}

function FlowEditor({
  initialNodes,
  initialEdges,
  isEditing,
  onSave,
  saving,
}: {
  initialNodes: Node<StepNodeData>[];
  initialEdges: Edge[];
  isEditing: boolean;
  onSave: (nodes: Node<StepNodeData>[]) => void;
  saving: boolean;
}) {
  const [nodes, setNodes, onNodesChange] = useNodesState(initialNodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState(initialEdges);
  const [editNodeId, setEditNodeId] = useState<string | null>(null);
  const [sheetOpen, setSheetOpen] = useState(false);
  const { fitView } = useReactFlow();

  const editNode = useMemo(() => nodes.find((n) => n.id === editNodeId) ?? null, [nodes, editNodeId]);

  const onConnect = useCallback(
    (params: Connection) => setEdges((eds) => addEdge(params, eds)),
    [setEdges],
  );

  const addNode = useCallback(
    (type: string) => {
      const existingNodes = nodes.filter((n) => !n.id.includes("_branch_"));
      const maxX = existingNodes.reduce((max, n) => Math.max(max, n.position.x), 0);
      const id = `step_${Date.now()}`;
      const newNode: Node<StepNodeData> = {
        id,
        type: "step",
        position: { x: maxX + 280, y: 200 },
        data: { title: `${type} step`, stepType: type, config: {}, next: "" },
      };
      const lastNode = existingNodes[existingNodes.length - 1];
      setNodes((nds) => [...nds, newNode]);
      if (lastNode) {
        setEdges((eds) => [...eds, { id: `e-${lastNode.id}-${id}`, source: lastNode.id, target: id }]);
      }
    },
    [nodes, setNodes, setEdges],
  );

  const handleNodeEdit = useCallback((id: string) => {
    setEditNodeId(id);
    setSheetOpen(true);
  }, []);

  const handleNodeSave = useCallback(
    (id: string, data: Partial<StepNodeData>) => {
      setNodes((nds) =>
        nds.map((n) => (n.id === id ? { ...n, data: { ...n.data, ...data } } : n)),
      );
    },
    [setNodes],
  );

  const handleNodeDelete = useCallback(
    (id: string) => {
      setNodes((nds) => nds.filter((n) => n.id !== id));
      setEdges((eds) => eds.filter((e) => e.source !== id && e.target !== id));
    },
    [setNodes, setEdges],
  );

  useEffect(() => {
    const handler = (e: Event) => {
      const custom = e as CustomEvent<{ id: string }>;
      handleNodeEdit(custom.detail.id);
    };
    window.addEventListener("workflow:edit-node", handler);
    return () => window.removeEventListener("workflow:edit-node", handler);
  }, [handleNodeEdit]);

  return (
    <div className="flex flex-col h-full">
      {!isEditing && (
        <div className="flex items-center gap-1 px-4 py-2 border-b bg-white">
          {STEP_TYPES.map(({ type, label, Icon, color }) => (
            <Button key={type} variant="outline" size="sm" onClick={() => addNode(type)}>
              <Plus className="size-3.5" style={{ color }} />
              <span className="ml-1">{label}</span>
            </Button>
          ))}
          <div className="flex-1" />
          <Button size="sm" onClick={() => onSave(nodes)} disabled={saving}>
            <Save className="size-4" />
            {saving ? "Saving..." : "Save"}
          </Button>
        </div>
      )}
      <div className="flex-1">
        <ReactFlow
          nodes={nodes}
          edges={edges}
          onNodesChange={onNodesChange}
          onEdgesChange={onEdgesChange}
          onConnect={onConnect}
          nodeTypes={NODE_TYPES}
          onInit={() => fitView({ padding: 0.2 })}
          deleteKeyCode="Delete"
          fitView
        >
          <Background variant={BackgroundVariant.Dots} />
          <Controls />
          <MiniMap />
        </ReactFlow>
      </div>

      <NodeEditSheet
        node={editNode}
        open={sheetOpen}
        onOpenChange={setSheetOpen}
        onSave={handleNodeSave}
        onDelete={handleNodeDelete}
      />
    </div>
  );
}

export default function WorkflowEditorPage() {
  const router = useRouter();
  const searchParams = useSearchParams();
  const workflowId = searchParams.get("id");
  const [loading, setLoading] = useState(false);
  const [name, setName] = useState("");
  const [saving, setSaving] = useState(false);
  const [initialNodes, setInitialNodes] = useState<Node<StepNodeData>[]>([]);
  const [initialEdges, setInitialEdges] = useState<Edge[]>([]);
  const [ready, setReady] = useState(false);

  const isEditing = !!workflowId;

  const defaultNodes: Node<StepNodeData>[] = useMemo(
    () => [
      {
        id: "s1",
        type: "step",
        position: { x: 300, y: 200 },
        data: { title: "Step 1", stepType: "task", config: {}, next: "" },
      },
    ],
    [],
  );

  useEffect(() => {
    if (!workflowId) {
      setInitialNodes(defaultNodes);
      setInitialEdges([]);
      setReady(true);
      return;
    }
    setLoading(true);
    api
      .get<{ name: string; steps: string }>(`/admin/workflows/${workflowId}`)
      .then((wf) => {
        setName(wf.name);
        const steps = JSON.parse(wf.steps);
        const { nodes, edges } = stepsToNodes(steps);
        setInitialNodes(nodes);
        setInitialEdges(edges);
      })
      .catch((err) => {
        if (err instanceof ApiError) toast.error(err.message);
        else toast.error("Failed to load workflow");
      })
      .finally(() => {
        setLoading(false);
        setReady(true);
      });
  }, [workflowId, defaultNodes]);

  const handleSave = useCallback(
    async (nodes: Node<StepNodeData>[]) => {
      if (isEditing) {
        toast.info("Workflow definitions are immutable. Create a new one to change steps.");
        return;
      }
      setSaving(true);
      try {
        const steps = nodesToSteps(nodes);
        const id = `wf-${Date.now()}`;
        await api.post("/admin/workflows", {
          id,
          name: name || "Untitled Workflow",
          steps,
        });
        toast.success("Workflow saved");
        router.push("/admin/workflows");
      } catch (err) {
        if (err instanceof ApiError) toast.error(err.message);
        else toast.error("Failed to save workflow");
      } finally {
        setSaving(false);
      }
    },
    [isEditing, name, router],
  );

  return (
    <div className="flex flex-col h-[calc(100vh-6rem)]">
      <div className="flex items-center justify-between border-b px-4 py-2">
        <div className="flex items-center gap-3">
          <Link href="/admin/workflows">
            <Button variant="ghost" size="icon-sm">
              <ArrowLeft className="size-4" />
            </Button>
          </Link>
          {isEditing ? (
            <h1 className="text-lg font-semibold">{name}</h1>
          ) : (
            <input
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="Workflow name..."
              className="text-lg font-semibold bg-transparent border-none outline-none w-64"
            />
          )}
        </div>
        {isEditing && (
          <Button variant="outline" size="sm" onClick={() => router.push("/admin/workflows")}>
            Back to list
          </Button>
        )}
      </div>

      <div className="flex-1 relative">
        {loading || !ready ? (
          <div className="flex items-center justify-center h-full">
            <div className="size-8 animate-spin rounded-full border-2 border-muted border-t-transparent" />
          </div>
        ) : (
          <ReactFlowProvider>
            <FlowEditor
              initialNodes={initialNodes}
              initialEdges={initialEdges}
              isEditing={isEditing}
              onSave={handleSave}
              saving={saving}
            />
          </ReactFlowProvider>
        )}
      </div>
    </div>
  );
}
