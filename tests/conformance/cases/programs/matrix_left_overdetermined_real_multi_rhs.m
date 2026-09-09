coefficients = [1, 0; 0, 1; 1, 1; 2, -1];
right_hand_side = [1, 2; 2, -1; 2, 4; 0, 3];

openmat_result = coefficients \ right_hand_side;
