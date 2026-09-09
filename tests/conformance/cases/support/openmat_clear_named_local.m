function result = openmat_clear_named_local()
first = 1;
second = 2;
clear first;
first = 3;
result = [first, second];
end
