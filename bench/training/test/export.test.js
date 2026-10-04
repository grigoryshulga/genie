import assert from "node:assert/strict";
import { test } from "node:test";

import { CSV_HEADER, exportCsv } from "../src/export.js";

test("the export is a header and one line per item", () => {
  const csv = exportCsv([{ name: "Огурцы", quantity: 2, price: 10, discount: 10 }]);
  assert.deepEqual(csv.split("\n"), [CSV_HEADER, "Огурцы;2;10;18"]);
});

test("an empty inventory is the header alone", () => {
  assert.equal(exportCsv([]), CSV_HEADER);
});
