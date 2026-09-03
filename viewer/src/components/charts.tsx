import {
  Bar,
  BarChart,
  Cell,
  CartesianGrid,
  Legend,
  Line,
  LineChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { KERNELS, type Results, kernelOrder } from "@/types";

const axis = { stroke: "hsl(var(--muted-foreground))", fontSize: 12 };

const tooltipStyle = {
  background: "hsl(var(--card))",
  border: "1px solid hsl(var(--border))",
  borderRadius: 8,
  fontSize: 12,
};

const fmt = (v: number | null) => (v == null ? "n/a" : `${v.toFixed(3)} ms`);

/**
 * Log-scale line chart of time vs opening count.
 *
 * Log scale is not decoration: the kernels span ~5 orders of magnitude
 * (0.002ms to 990ms), so a linear axis would flatten every Rust column into
 * the baseline and show only CGAL and OCCT.
 */
export function ScalingChart({ data }: { data: Results }) {
  const keys = kernelOrder(data.built);
  return (
    <ResponsiveContainer width="100%" height={380}>
      <LineChart data={data.rows} margin={{ top: 8, right: 16, bottom: 8, left: 0 }}>
        <CartesianGrid stroke="hsl(var(--border))" strokeDasharray="3 3" />
        <XAxis
          dataKey="n"
          {...axis}
          label={{ value: "openings", position: "insideBottom", offset: -4, fill: axis.stroke }}
        />
        <YAxis
          scale="log"
          domain={["auto", "auto"]}
          {...axis}
          tickFormatter={(v: number) => (v >= 1 ? `${v}` : v.toString())}
          label={{ value: "ms (log)", angle: -90, position: "insideLeft", fill: axis.stroke }}
        />
        <Tooltip contentStyle={tooltipStyle} formatter={(v) => fmt(v as number)} />
        <Legend wrapperStyle={{ fontSize: 12 }} />
        {keys.map((k) => (
          <Line
            key={k}
            type="monotone"
            dataKey={k}
            name={KERNELS[k].label}
            stroke={KERNELS[k].color}
            strokeWidth={KERNELS[k].kind === "analytic" ? 2.5 : 1.8}
            strokeDasharray={KERNELS[k].kind === "analytic" ? undefined : "4 3"}
            dot={{ r: 2 }}
            connectNulls={false}
          />
        ))}
      </LineChart>
    </ResponsiveContainer>
  );
}

/** Grouped bars at a single opening count — easier for direct comparison. */
export function ComparisonChart({ data, n }: { data: Results; n: number }) {
  const row = data.rows.find((r) => r.n === n);
  if (!row) return null;
  const bars = kernelOrder(data.built)
    .map((k) => ({ kernel: KERNELS[k].label, ms: row[k] as number | null, fill: KERNELS[k].color }))
    .filter((b) => b.ms != null);

  return (
    <ResponsiveContainer width="100%" height={380}>
      <BarChart data={bars} layout="vertical" margin={{ top: 8, right: 48, bottom: 8, left: 8 }}>
        <CartesianGrid stroke="hsl(var(--border))" strokeDasharray="3 3" horizontal={false} />
        <XAxis type="number" scale="log" domain={["auto", "auto"]} {...axis} />
        <YAxis type="category" dataKey="kernel" width={130} {...axis} />
        <Tooltip contentStyle={tooltipStyle} formatter={(v) => fmt(v as number)} />
        <Bar dataKey="ms" radius={[0, 4, 4, 0]}>
          {bars.map((b) => (
            <Cell key={b.kernel} fill={b.fill} />
          ))}
        </Bar>
      </BarChart>
    </ResponsiveContainer>
  );
}
