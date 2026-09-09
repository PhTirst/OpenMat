classdef OpenMatBaseNumber
    properties
        Value
    end

    methods
        function object = OpenMatBaseNumber(value)
            object.Value = value;
        end

        function result = score(object)
            result = object.Value;
        end

        function result = describe(object)
            result = [object.Value, object.score()];
        end
    end
end
