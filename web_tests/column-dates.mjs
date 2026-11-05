import test from "node:test";
import assert from "node:assert/strict";
import {datePart,filterValue} from "../src/web_assets/columns.js";

test("year-month-day timestamp supports exact, date and month filtering",()=>{
  const type={dataType:"Date/Time",dateOrder:"YMD_HMS"};
  const value="2026-10-29 14:05:09";
  assert.equal(filterValue(value,type,"exact"),value);
  assert.equal(filterValue(value,type,"date"),"2026-10-29");
  assert.equal(filterValue(value,type,"month"),"2026-10");
  assert.equal(datePart("2024-02-29 00:00:00",type.dateOrder),"2024-02-29");
  for(const invalid of ["2026-02-29 10:00:00","2026-10-29 24:00:00","2026-10-29 10:60:00","2026-10-29 10:00:60","29/10/2026 10:00:00","2026-10-29","0000-01-01 00:00:00"]){
    assert.equal(datePart(invalid,type.dateOrder),null,invalid);
    assert.equal(filterValue(invalid,type,"date"),invalid);
  }
  assert.equal(datePart("03/04/2026","DMY"),"2026-04-03");
  assert.equal(datePart("03/04/2026","MDY"),"2026-03-04");
});
