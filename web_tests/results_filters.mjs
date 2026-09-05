import assert from "node:assert/strict";
import test from "node:test";

import { filteredIncidentCount, pivotCellFilters, treeVisibleRows } from "../src/web_assets/results.js";
import { state } from "../src/web_assets/state.js";

function setResultState() {
  state.analysis = {
    processed_incidents: [0, 1, 2, 3].map((source_row_index) => ({ source_row_index })),
    source: {
      rows: [
        ["Open", "Zurich"],
        ["Closed", "Zurich"],
        ["Open", "Bern"],
        ["Open", "Zurich"],
      ],
    },
  };
  state.detailColumnFilters = [
    { selected: null, query: "", searchDeselected: false },
    { selected: null, query: "", searchDeselected: false },
  ];
  state.detailDrilldownRowIndices = null;
}

test.beforeEach(setResultState);

test("tree rows are unrestricted when no detail constraint is active", () => {
  assert.equal(treeVisibleRows(), null);
});

test("tree rows apply detail filters across all processed records", () => {
  state.detailColumnFilters[0].selected = ["Open"];

  const visibleRows = treeVisibleRows();

  assert.deepEqual([...visibleRows], [0, 2, 3]);
  assert.equal(filteredIncidentCount([0, 1, 2], visibleRows), 2);
  assert.equal(filteredIncidentCount([1], visibleRows), 0);
});

test("tree rows combine detail filters with pivot drilldown rows", () => {
  state.detailColumnFilters[0].selected = ["Open"];
  state.detailColumnFilters[1].selected = ["Zurich"];
  state.detailDrilldownRowIndices = [1, 3];

  assert.deepEqual([...treeVisibleRows()], [3]);
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
