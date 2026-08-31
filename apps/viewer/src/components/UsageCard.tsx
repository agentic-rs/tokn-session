import type { UsageCardSummary } from "../lib/types";
import type { TechnicalCardHeading } from "./CardPresentation";

interface UsageCardProps {
  usage: UsageCardSummary;
}

const tokenFormatter = new Intl.NumberFormat("en-US");

function usageKindLabel(kind: string): string {
  switch (kind) {
    case "model_call":
      return "Model call";
    case "operation_total":
      return "Operation total";
    case "session_snapshot":
      return "Session snapshot";
    default:
      return "Provider usage";
  }
}

export function formatTokenCount(value: string): string {
  try {
    return tokenFormatter.format(BigInt(value));
  } catch {
    return value;
  }
}

export function usageHeading(usage: UsageCardSummary): TechnicalCardHeading {
  const inputOutput = `${formatTokenCount(usage.input_tokens)} input · ${formatTokenCount(usage.output_tokens)} output`;
  return {
    action: "Usage",
    primary: usage.total_tokens === null ? inputOutput : `${formatTokenCount(usage.total_tokens)} tokens`,
    secondary: `${usageKindLabel(usage.kind)} · ${inputOutput}`,
    monospace: false,
  };
}

function Metric({ label, value }: { label: string; value: string | null }) {
  if (value === null) {
    return null;
  }
  return (
    <div>
      <dt>{label}</dt>
      <dd title={value}>{formatTokenCount(value)}</dd>
    </div>
  );
}

function scopeNotice(kind: string): string | null {
  switch (kind) {
    case "session_snapshot":
      return "This cumulative session snapshot replaces earlier snapshots; do not sum these rows.";
    case "operation_total":
      return "This total belongs to the recorded operation, not a new model call.";
    default:
      return null;
  }
}

export function UsageCard({ usage }: UsageCardProps) {
  const hasBreakdown = usage.cache_read_tokens !== null
    || usage.cache_write_tokens !== null
    || usage.reasoning_tokens !== null;
  const notice = scopeNotice(usage.kind);

  return (
    <section className="usage-card" aria-label="Usage breakdown">
      <dl className="usage-card__metrics">
        <Metric label="Input" value={usage.input_tokens} />
        <Metric label="Output" value={usage.output_tokens} />
        <Metric label="Reported total" value={usage.total_tokens} />
        <Metric label="Cache read" value={usage.cache_read_tokens} />
        <Metric label="Cache write" value={usage.cache_write_tokens} />
        <Metric label="Reasoning" value={usage.reasoning_tokens} />
      </dl>
      {hasBreakdown ? (
        <p className="usage-card__notice">
          Cache counts are already included in input. Reasoning is a provider-reported breakdown;
          totals remain provider-reported.
        </p>
      ) : null}
      {notice ? <p className="usage-card__notice">{notice}</p> : null}
    </section>
  );
}
