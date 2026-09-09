classdef OpenMatHandleCounter < handle
    properties
        Value
    end

    methods
        function object = OpenMatHandleCounter(initialValue)
            object.Value = initialValue;
        end

        function bump(object, amount)
            object.Value = object.Value + amount;
        end
    end
end
