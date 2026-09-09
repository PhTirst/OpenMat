coefficients = complex(single([2, 1; 0, 3]), ...
    single([1, 0; -1, 2]));
right_hand_side = complex(single([1; 2]), single([2; -1]));

openmat_result = coefficients \ right_hand_side;
