a = regexprep("abc123def", '(\d+)', '<$1>');
b = regexprep("AaA", 'a', 'x', 'ignorecase');
c = regexprep("a1b22", '\d+', '#', 'once');
d = regexprep("abc", '', ':', 'emptymatch');
matches = regexp("a1b22", '\d+', 'match');
pieces = regexp("ab12 cd345", '[a-z]+\d+', 'split');
openmat_result = [a, b, c, d, matches, pieces];
