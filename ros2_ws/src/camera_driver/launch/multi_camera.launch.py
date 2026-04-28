from launch import LaunchDescription
from launch_ros.actions import Node
from ament_index_python.packages import get_package_share_directory
import os


def generate_launch_description():

    config = os.path.join(
        get_package_share_directory("camera_driver"),
        "config",
        "cameras.yaml"
    )

    return LaunchDescription([

        Node(
            package="camera_driver",
            executable="camera_manager_node",
            parameters=[config],
            output="screen"
        )

    ])