first = OpenMatEnumHandle.Only;
second = OpenMatEnumHandle.Only;
delete(first);
third = OpenMatEnumHandle.Only;
openmat_result = [isvalid(first), isvalid(second), isvalid(third)];
