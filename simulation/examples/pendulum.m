function dx = pendulum(x, p)
% x = [angle; angular velocity], p = [gravity/length; damping].
omega = x(2);
acceleration = -p(1) * sin(x(1)) - p(2) * omega;
dx = [omega; acceleration];
end
