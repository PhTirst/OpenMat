classdef OpenMatAddend
    properties
        Value
    end

    methods
        function object = OpenMatAddend(value)
            object.Value = value;
        end

        function result = plus(left, right)
            result = left.Value + right.Value;
        end
    end
end
