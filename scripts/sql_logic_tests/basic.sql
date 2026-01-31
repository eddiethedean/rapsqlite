# Minimal SQL logic test for rapsqlite (simple format: SQL block then "----" then expected rows).
# Runner runs each statement (semicolon-separated); after "----" lines are expected result rows (tab-separated).

CREATE TABLE t (id INTEGER PRIMARY KEY, x TEXT);
INSERT INTO t (id, x) VALUES (1, 'a'), (2, 'b');
SELECT id, x FROM t ORDER BY id
----
1	a
2	b
