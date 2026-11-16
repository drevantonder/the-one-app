---
name: sheets
description: Read before reading the person's Google Sheets, or adding rows, changing cells or making a spreadsheet.
---

Google Sheets is the person's spreadsheets. Signing in is the skill `google`'s.

- `spreadsheet` lists a spreadsheet's sheets (tabs). `read` reads a range in A1 notation, like Budget!A1:F20,
  the first sheet if left out.
- `append` adds rows after a table, `update` writes over a range and `create` makes a spreadsheet: change only
  what the person asked. Values are read as if typed, so =SUM(A1:A3) is a formula.
- A spreadsheet is given as `spreadsheetId`, its id or its address. `drive_files_list` finds one.
- With several accounts, each tool takes `account`. `accounts` lists them.
