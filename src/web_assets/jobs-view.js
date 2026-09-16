export function jobSummaryText(job) {
  const summary=job.metadata?.summary||{};
  const count=value=>Number.isInteger(value)?value.toLocaleString():"Not recorded";
  const rows=Number.isInteger(summary.processedRows)?`Rows processed: ${count(summary.processedRows)} of ${count(summary.sourceRows)}`:`Source rows: ${count(summary.sourceRows)}`;
  return `Source: ${summary.sourceFileName||"Not recorded"} · ${rows} · Columns: ${count(summary.columnCount)}`;
}
export function jobAction(job) {
  return ["queued","running"].includes(job.state)?"cancel":["failed","cancelled"].includes(job.state)?"resubmit":null;
}
