classdef OpenMatDerivedNumber < OpenMatBaseNumber
    methods
        function object = OpenMatDerivedNumber(value)
            object@OpenMatBaseNumber(value);
        end

        function result = score(object)
            result = object.Value * 3;
        end
    end
end
