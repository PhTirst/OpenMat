first_counter = openmat_nested_counter(10);
second_counter = openmat_nested_counter(100);

first = first_counter(1);
second = first_counter(2);
independent = second_counter(3);
surviving = first_counter(4);

openmat_result = [first, second, independent, surviving];
