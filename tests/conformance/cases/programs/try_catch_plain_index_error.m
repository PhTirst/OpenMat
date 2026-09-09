caught = false;
reached_after_error = false;
try
    value = 1;
    value(2);
    reached_after_error = true;
catch
    caught = true;
end
openmat_result = [caught, reached_after_error];
