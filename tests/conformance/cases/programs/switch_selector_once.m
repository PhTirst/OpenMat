global openmat_switch_selector_count;
openmat_switch_selector_count = 0;

selected = 0;
switch openmat_switch_selector()
    case 1
        selected = 10;
    case 2
        selected = 20;
    otherwise
        selected = 30;
end

unmatched = 0;
switch 9
    case 1
        unmatched = 1;
end

openmat_result = [selected, openmat_switch_selector_count, unmatched];

function value = openmat_switch_selector()
global openmat_switch_selector_count;
openmat_switch_selector_count = openmat_switch_selector_count + 1;
value = 2;
end
