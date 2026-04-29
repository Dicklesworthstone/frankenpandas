-- mode5 indexed t2 query
SELECT k, v FROM t2 WHERE k IN ('x', 'z') ORDER BY k;
