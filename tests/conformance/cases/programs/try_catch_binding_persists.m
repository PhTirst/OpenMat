caught = false;
try
    value = 1;
    value(2);
catch caught_exception
    caught = true;
end
openmat_result = [caught, isobject(caught_exception), ...
    numel(caught_exception) == 1];
