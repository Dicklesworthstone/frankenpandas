SELECT t1.a, t1.b, t2.v FROM t1 JOIN t2 ON t2.k = CASE t1.a WHEN 1 THEN 'x' WHEN 2 THEN 'y' ELSE 'z' END ORDER BY t1.a;
