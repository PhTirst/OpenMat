a = sprintf('%g,', [1 2; 3 4]);
b = sprintf('%*.*f', 8, 3, 1.25);
c = sprintf(string('%04d'), 12);
d = num2str(pi);
e = num2str([1 2 3]);
f = num2str([1.23 0], '%.2f');
openmat_result = [string(a), string(b), c, string(d), string(e), string(f)];
