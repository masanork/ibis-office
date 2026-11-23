# Monthly report check

This example checks a synthetic report for duplicate entries by month and department. Use the same workflow for a user workbook by first confirming its scope, business key, and allowed threshold.

1. Use the bundled [sample CSV](../assets/monthly-report.csv) or copy it into a directory the file MCP server is allowed to read.
2. Call `sqlite_describe_datasets` with `{"datasets":[{"source":"/absolute/path/to/monthly-report.csv","alias":"report"}]}`.
3. Run the query below with `sqlite_query`, using the same dataset binding.
4. Report the number of duplicate groups and whether it exceeds the permitted count of zero. Include the month, department, and row count for each group. Keep a workbook unchanged when the request is to check it.

```sql
SELECT report_month, department, COUNT(*) AS row_count
FROM report
GROUP BY report_month, department
HAVING COUNT(*) > 1
ORDER BY report_month, department;
```

The sample has one duplicate group, `2026-10 / Sales`, with two rows, so it fails the threshold of zero. If a workbook already has a saved query report, start with `xlsx_list_query_reports` to inspect its registered checks before proposing any changes. Apply the user's requested scope when choosing between running a read-only query and updating a report or its checks.
