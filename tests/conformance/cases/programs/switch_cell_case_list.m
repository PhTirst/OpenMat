string_match = 0;
switch "beta"
    case {"alpha", "beta"}
        string_match = 1;
    otherwise
        string_match = -1;
end

cross_text_match = 0;
switch 'same'
    case "same"
        cross_text_match = 1;
    otherwise
        cross_text_match = -1;
end

first_match = 0;
switch 3
    case 3
        first_match = 1;
    case 3
        first_match = 2;
end

openmat_result = [string_match, cross_text_match, first_match];
