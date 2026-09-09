total = 0;
visits = 0;
for value = 1:6
    visits = visits + 1;
    if value == 2
        continue;
    end
    if value == 5
        break;
    end
    total = total + value;
end

empty_visits = 0;
for ignored = 3:2
    empty_visits = empty_visits + 1;
end

openmat_result = [visits, total, empty_visits];
