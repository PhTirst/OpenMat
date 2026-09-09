offset = 11;
values = [2, 4, 6];
handle = @(index) values(index) + offset;
offset = 100;
values(2) = 40;
openmat_result = handle(2);
