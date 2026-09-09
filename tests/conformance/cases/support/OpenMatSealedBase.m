classdef (Sealed) OpenMatSealedBase
    methods
        function value = calculate(object, input)
            value = input * 6;
        end
    end
end
