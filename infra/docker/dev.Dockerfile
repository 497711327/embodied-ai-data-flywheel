FROM ubuntu:22.04

RUN apt update && apt install -y curl gnupg lsb-release

# ROS2 repo
RUN curl -sSL https://raw.githubusercontent.com/ros/rosdistro/master/ros.key | apt-key add -
RUN echo "deb http://packages.ros.org/ros2/ubuntu jammy main" > /etc/apt/sources.list.d/ros2.list

RUN apt update && apt install -y ros-humble-desktop

RUN echo "source /opt/ros/humble/setup.bash" >> ~/.bashrc