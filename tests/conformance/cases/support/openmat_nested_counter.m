function counter = openmat_nested_counter(seed)
count = seed;
counter = @increment;

    function value = increment(delta)
        count = count + delta;
        value = count;
    end
end
