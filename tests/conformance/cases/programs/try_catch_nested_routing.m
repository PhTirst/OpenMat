trace = 0;
try
    try
        value = 1;
        value(2);
    catch
        trace = trace + 1;
        other_value = 1;
        other_value(2);
    end
    trace = 100;
catch
    trace = trace + 10;
end
openmat_result = trace;
