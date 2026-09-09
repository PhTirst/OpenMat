[starts, ends] = regexp('ab12 cd345', '(?<letters>[a-z]+)(?<digits>\d+)');
empty_matches = regexp('abc', '', 'emptymatch');
folded = regexpi('aA', 'a');
lookbehind_backreference = regexp('ab22 c33', '(?<=b)(\d)\1');
selected_ends = regexp('a1b22', '\d+', 'end');
openmat_result = [starts, ends, empty_matches, folded, ...
    lookbehind_backreference, selected_ends];
