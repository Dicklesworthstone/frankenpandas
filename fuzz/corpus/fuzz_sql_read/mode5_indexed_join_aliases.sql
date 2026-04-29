-- mode5 indexed join query
SELECT t2.k, t1.b, t2.v FROM t2 JOIN t1 ON t1.a = CAST(t2.v - 0.5 AS INTEGER) ORDER BY t2.k;
