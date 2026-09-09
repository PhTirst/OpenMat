[single_value, single_index] = min(single([3 1; 2 0]));
integer_value = min(int16([3 1]));
logical_value = min(logical([1 0]));
char_value = min('AZ');
mixed_single = min(single([1 4]), [2 3]);
integer_elementwise = min(int8([3 1]), int8([2 4]));
char_elementwise = min('AZ', 'BY');
openmat_result = [string(class(single_value)), string(class(single_index)), ...
    string(class(integer_value)), string(class(logical_value)), string(class(char_value)), ...
    string(class(mixed_single)), string(class(integer_elementwise)), ...
    string(class(char_elementwise))];
