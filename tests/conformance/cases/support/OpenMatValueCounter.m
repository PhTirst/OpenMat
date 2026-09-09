classdef OpenMatValueCounter
    properties
        Value
    end

    methods
        function object = OpenMatValueCounter(initialValue)
            object.Value = initialValue;
        end

        function object = bump(object, amount)
            object.Value = object.Value + amount;
        end
    end
end
