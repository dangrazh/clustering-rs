import assert from "node:assert/strict";
import test from "node:test";

import { detailFilterKey, filteredIncidentCount, pivotCellFilters, treeVisibleRows } from "../src/web_assets/results.js";
import { state } from "../src/web_assets/state.js";

function setResultState() {
  state.analysis = {
    processed_row_indices: [0, 1, 2, 3],
    source: { headers: ["State", "Site"], row_count: 4 },
  };
  state.rowData = null;
  state.detailColumnFilters = [
    { selected: null, query: "", searchDeselected: false },
    { selected: null, query: "", searchDeselected: false },
  ];
  state.detailDrilldownRowIndices = null;
}

// Row text lives on the server; this stands in for its answer to the current column filters.
const rows = [
  ["Open", "Zurich"],
  ["Closed", "Zurich"],
  ["Open", "Bern"],
  ["Open", "Zurich"],
];
function serverMatched() {
  const filters = JSON.parse(detailFilterKey());
  const match = rows.flatMap((row, index) => (filters.every((f) => f.selected.includes(row[f.column])) ? [index] : []));
  state.rowData = { filterKey: detailFilterKey(), match: new Set(match), sortColumn: null, ranks: null, page: new Map(), valuesKey: null, values: [] };
}

test.beforeEach(setResultState);

test("tree rows are unrestricted when no detail constraint is active", () => {
  assert.equal(treeVisibleRows(), null);
});

test("tree rows apply detail filters across all processed records", () => {
  state.detailColumnFilters[0].selected = ["Open"];
  serverMatched();

  const visibleRows = treeVisibleRows();

  assert.deepEqual([...visibleRows], [0, 2, 3]);
  assert.equal(filteredIncidentCount([0, 1, 2], visibleRows), 2);
  assert.equal(filteredIncidentCount([1], visibleRows), 0);
});

test("tree rows combine detail filters with pivot drilldown rows", () => {
  state.detailColumnFilters[0].selected = ["Open"];
  state.detailColumnFilters[1].selected = ["Zurich"];
  state.detailDrilldownRowIndices = [1, 3];
  serverMatched();

  assert.deepEqual([...treeVisibleRows()], [3]);
});

test("no row counts as matching until the server answered for the current filters", () => {
  state.detailColumnFilters[0].selected = ["Open"];
  assert.deepEqual([...treeVisibleRows()], []);
  serverMatched();
  state.detailColumnFilters[0].selected = ["Closed"];
  assert.deepEqual([...treeVisibleRows()], []);
});

test("row-only pivot values create row filters", () => {
  const pivot = {
    numericColumns: [2],
    columnFilterValues: [],
    rows: [{ total: false, rowFilterValues: [["Switzerland"], ["Zurich"]] }],
  };

  assert.deepEqual(pivotCellFilters(pivot, 0, 2, [0, 1], []), [
    { column: 0, selected: ["Switzerland"] },
    { column: 1, selected: ["Zurich"] },
  ]);
});

test("column-only pivot values create column filters but totals do not", () => {
  const pivot = {
    numericColumns: [1, 2],
    columnFilterValues: [[["Open"]]],
    rows: [{ total: false, rowFilterValues: [] }],
  };

  assert.deepEqual(pivotCellFilters(pivot, 0, 1, [], [2]), [{ column: 2, selected: ["Open"] }]);
  assert.deepEqual(pivotCellFilters(pivot, 0, 2, [], [2]), []);
});

test("combined pivot cells apply dimensions represented by detail and total cells", () => {
  const pivot = {
    numericColumns: [1, 2],
    columnFilterValues: [[["Open"]]],
    rows: [
      { total: false, rowFilterValues: [["Zurich"]] },
      { total: true, rowFilterValues: [] },
    ],
  };

  assert.deepEqual(pivotCellFilters(pivot, 0, 1, [0], [2]), [
    { column: 0, selected: ["Zurich"] },
    { column: 2, selected: ["Open"] },
  ]);
  assert.deepEqual(pivotCellFilters(pivot, 0, 2, [0], [2]), [{ column: 0, selected: ["Zurich"] }]);
  assert.deepEqual(pivotCellFilters(pivot, 1, 1, [0], [2]), [{ column: 2, selected: ["Open"] }]);
  assert.deepEqual(pivotCellFilters(pivot, 1, 2, [0], [2]), []);
});

test("reviewer selection intersects column and drilldown filters", () => {
  state.analysis.clusters = [
    { id: 1, incident_row_indices: [0, 1], subgroups: [] },
    { id: 2, incident_row_indices: [2, 3], subgroups: [] },
  ];
  state.shared = { reviewers: { 1: { value: { userId: "alice" } }, 2: { value: { userId: "bob" } } } };
  state.reviewerFilter = { selected: true, users: ["bob"] };
  state.workflowStates = [];
  state.detailColumnFilters[1] = { selected: ["Zurich"], query: "", searchDeselected: false };
  serverMatched();
  assert.deepEqual([...treeVisibleRows()], [3]);
  state.detailDrilldownRowIndices = [2];
  assert.equal(treeVisibleRows().size, 0);
  state.reviewerFilter = {};
  state.shared = null;
});
