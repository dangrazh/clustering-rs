import { state } from "./state.js";

export function assignment(s, cluster) {
  const entry = s.shared?.reviewers?.[String(cluster)] || s.reviewers?.[String(cluster)];
  return entry?.value || entry || {};
}

export function matchesReviewer(reviewer, filter, userId) {
  if (!filter?.selected) return true;
  if (reviewer.unconfirmed) return !!filter.unconfirmed;
  if (!reviewer.userId) return !!filter.unassigned;
  return !!(filter.users?.includes(reviewer.userId) || filter.me && reviewer.userId === userId);
}

export function reviewerRows(s) {
  if (!s.reviewerFilter?.selected) return null;
  const rows = new Set();
  for (const cluster of s.analysis?.clusters || []) {
    if (matchesReviewer(assignment(s, cluster.id), s.reviewerFilter, s.user?.id)) {
      for (const row of cluster.incident_row_indices) rows.add(row);
    }
  }
  return rows;
}

export function reviewerLabel(cluster) {
  const reviewer = assignment(state, cluster);
  const span = document.createElement("span");
  span.className = "tree-reviewer";
  span.textContent = reviewer.unconfirmed
    ? `${reviewer.recorded?.name || "Imported reviewer"} (unconfirmed)`
    : reviewer.userId
      ? state.users?.find(user => user.id === reviewer.userId)?.name || reviewer.recorded?.name || "Reviewer"
      : "Unassigned";
  span.title = span.textContent;
  return span;
}

export function bindReviewerFilter(changed) {
  const panel = document.createElement("details");
  panel.className = "reviewer-filter";
  const summary = document.createElement("summary");
  panel.append(summary);
  const choices = document.createElement("div");
  choices.className = "reviewer-choices";
  panel.append(choices);
  // Filters belong outside the fixed tree / splitter / detail grid.
  document.querySelector("#results .workflow-filters").after(panel);

  function render() {
    const filter = state.reviewerFilter || {};
    const focus = choices.contains(document.activeElement) ? document.activeElement.dataset.filterKey : null;
    const scroll = choices.scrollTop;
    summary.textContent = filter.selected ? "Cluster reviewer: Filtered" : "Cluster reviewer: All";
    choices.replaceChildren();
    const all = document.createElement("button");
    all.textContent = "All";
    all.type = "button";
    all.dataset.filterKey = "all";
    all.onclick = () => { state.reviewerFilter = {}; changed(); };
    choices.append(all);

    const users = new Map((state.users || []).map(user => [user.id, user.name]));
    // Keep historical confirmed assignments selectable even if the user is no longer active.
    for (const cluster of state.analysis?.clusters || []) {
      const reviewer = assignment(state, cluster.id);
      if (reviewer.userId && !reviewer.unconfirmed && !users.has(reviewer.userId)) {
        users.set(reviewer.userId, reviewer.recorded?.name || "Reviewer");
      }
    }
    const options = [
      ["me", "Assigned to me"], ["unassigned", "Unassigned"],
      ["unconfirmed", "Unconfirmed imported reviewer"], ...users,
    ];
    for (const [key, name] of options) {
      const label = document.createElement("label"), input = document.createElement("input");
      input.type = "checkbox";
      input.dataset.filterKey = key;
      const category = ["me", "unassigned", "unconfirmed"].includes(key);
      input.checked = !!(filter.selected && (category ? filter[key] : filter.users?.includes(key)));
      label.append(input, document.createTextNode(name));
      choices.append(label);
      input.onchange = () => {
        const next = structuredClone(state.reviewerFilter || {});
        next.selected = true;
        if (category) next[key] = input.checked;
        else next.users = input.checked
          ? [...new Set([...(next.users || []), key])]
          : (next.users || []).filter(id => id !== key);
        state.reviewerFilter = next;
        changed();
      };
    }
    choices.scrollTop = scroll;
    if (focus) [...choices.querySelectorAll("[data-filter-key]")]
      .find(element => element.dataset.filterKey === focus)?.focus({ preventScroll: true });
  }
  window.addEventListener("reviewer-render", render);
  render();
}
