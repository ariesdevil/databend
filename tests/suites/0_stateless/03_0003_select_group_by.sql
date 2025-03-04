SELECT max(number) from numbers_mt(0) group by number % 4;
SELECT max(number) FROM numbers_mt (10) where number > 99999999998 GROUP BY number%3;
SELECT avg(number), max(number+1)+1 FROM numbers_mt(10000) where number > 2 GROUP BY 1;
SELECT number%3 as c1, number%2 as c2 FROM numbers_mt(10000) where number > 2 group by number%3, number%2 order by c1,c2;

SELECT number%3 as c1 FROM numbers_mt(10) where number > 2 group by number%3 order by c1;

SELECT 'NOT in GROUP BY function check';
-- SELECT number%3 as c1, number as c2 FROM numbers_mt(10) where number > 2 group by c1 order by c1;
